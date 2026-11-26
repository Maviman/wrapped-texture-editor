// Owner: core-document (C2). Undo / redo (ARCHITECTURE.md 5.18, algorithm 6.7): sparse tile patches for pixel edits,
// structure snapshots for everything else, the history stacks with their memory budget, and pending
// (`Commit::Preview`) edits.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use crate::document::Document;
use crate::doc_sticker_edits::StickerPatch;
use crate::ids::{LayerId, StampId, StickerId, StickerLayerId, TextureSetId};
use crate::layer::{BlendMode, Layer, LayerKind};
use crate::math::IRect;
use crate::pixel::{PixelBuf, SparseBuf, TileBuf};
use crate::stamp::{StampObject, StampSource};
use crate::sticker::StickerDoc;

/// Pixels of one tile of a `TilePatch` / `MaskPatch`.
pub enum TileData {
    /// Every pixel of the tile has this value: the colour of an RGBA tile; for a one-byte buffer (`MaskPatch`) the
    /// value is `[0]` and the other three bytes are 0.
    Solid([u8; 4]),
    /// `tile_w * tile_h * BPP` bytes (edge tiles are smaller).
    Raw(Box<[u8]>),
}

/// One tile of a `TilePatch`, addressed in tile coordinates.
pub struct Tile {
    pub tx: u16,
    pub ty: u16,
    pub data: TileData,
}

/// Sparse set of 64x64 tiles of one buffer with `BPP` bytes per pixel (STICKER_PLAN 2.10: generic over the bytes per
/// pixel, review A4). Applying it swaps its contents with the buffer (it becomes its own inverse). The patch knows
/// its kind from its type and refuses a buffer of another kind (`swap_with` changes nothing then).
pub struct TilePatchOf<const BPP: usize> {
    pub tiles: Vec<Tile>,
}

/// A patch of a 4-byte buffer (`PixelBuf`): paint layers (`Edit::Pixels`) and sticker pictures.
pub type TilePatch = TilePatchOf<4>;
/// A patch of a 1-byte buffer (`AlphaBuf`): sticker layer masks.
pub type MaskPatch = TilePatchOf<1>;

/// Tile side in pixels.
const TILE: u32 = 64;

/// Pixel rect of tile `(tx, ty)` in a `width x height` image; edge tiles are clipped, tiles outside are `EMPTY`.
fn tile_rect(tx: u16, ty: u16, width: u32, height: u32) -> IRect {
    let (x0, y0) = (tx as u32 * TILE, ty as u32 * TILE);
    if x0 >= width || y0 >= height {
        return IRect::EMPTY;
    }
    let (x1, y1) = ((x0 + TILE).min(width), (y0 + TILE).min(height));
    IRect::new(x0 as i32, y0 as i32, x1 as i32, y1 as i32)
}

/// The rectangle of a whole buffer.
fn buf_rect<B: TileBuf>(buf: &B) -> IRect {
    let [w, h] = buf.size();
    IRect::from_size(w, h)
}

/// Byte range of the part of row `y` that lies inside `r` (which must lie inside `buf`).
fn row_span<B: TileBuf>(buf: &B, r: IRect, y: i32) -> std::ops::Range<usize> {
    let start = (y as usize * buf.size()[0] as usize + r.x0 as usize) * B::BPP;
    start..start + r.width() as usize * B::BPP
}

fn read_tile<B: TileBuf>(buf: &B, r: IRect) -> Box<[u8]> {
    let mut out = Vec::with_capacity(r.area() as usize * B::BPP);
    for y in r.y0..r.y1 {
        out.extend_from_slice(&buf.bytes()[row_span(buf, r, y)]);
    }
    out.into_boxed_slice()
}

/// Writes `data` (`r.area() * BPP` bytes) into `buf` at `r`.
fn write_tile<B: TileBuf>(buf: &mut B, r: IRect, data: &[u8]) {
    let n = r.width() as usize * B::BPP;
    for (i, y) in (r.y0..r.y1).enumerate() {
        let span = row_span(buf, r, y);
        buf.bytes_mut()[span].copy_from_slice(&data[i * n..(i + 1) * n]);
    }
}

fn fill_tile<B: TileBuf>(buf: &mut B, r: IRect, value: [u8; 4]) {
    let value = &value[..B::BPP];
    for y in r.y0..r.y1 {
        let span = row_span(buf, r, y);
        for px in buf.bytes_mut()[span].chunks_exact_mut(B::BPP) {
            px.copy_from_slice(value);
        }
    }
}

fn tile_equals<B: TileBuf>(buf: &B, r: IRect, data: &[u8]) -> bool {
    let n = r.width() as usize * B::BPP;
    data.len() == r.area() as usize * B::BPP
        && (r.y0..r.y1).enumerate().all(|(i, y)| buf.bytes()[row_span(buf, r, y)] == data[i * n..(i + 1) * n])
}

/// `Solid` when every pixel is the same, else `Raw`.
fn pack_tile(data: Box<[u8]>, bpp: usize) -> TileData {
    let mut pixels = data.chunks_exact(bpp);
    match pixels.next() {
        Some(first) if pixels.all(|p| p == first) => {
            let mut solid = [0u8; 4];
            solid[..bpp].copy_from_slice(first);
            TileData::Solid(solid)
        }
        _ => TileData::Raw(data),
    }
}

impl<const BPP: usize> TilePatchOf<BPP> {
    /// Tile side in pixels.
    pub const TILE: u32 = TILE;

    /// An empty patch.
    pub fn empty() -> TilePatchOf<BPP> {
        TilePatchOf { tiles: Vec::new() }
    }

    /// Bytes per pixel of the buffers this patch is made for.
    pub fn bpp(&self) -> usize {
        BPP
    }

    /// True when the patch can be applied to a buffer of type `B` (same bytes per pixel).
    pub fn fits<B: TileBuf>(&self) -> bool {
        B::BPP == BPP
    }

    /// True when the patch holds no tile.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// The patch that turns `new` back into `old` (STICKER_PLAN 2.8 `replace_sticker_pixels`): the tiles of `old`
    /// where the two differ, in (row, column) order. Empty for buffers of another kind or of different sizes.
    pub fn diff<B: TileBuf>(old: &B, new: &B) -> TilePatchOf<BPP> {
        let [w, h] = old.size();
        if !(B::BPP == BPP && new.size() == [w, h]) {
            return TilePatchOf::empty();
        }
        let mut tiles = Vec::new();
        for ty in 0..h.div_ceil(TILE) {
            for tx in 0..w.div_ceil(TILE) {
                let r = tile_rect(tx as u16, ty as u16, w, h);
                if r.is_empty() || (r.y0..r.y1).all(|y| old.bytes()[row_span(old, r, y)] == new.bytes()[row_span(new, r, y)]) {
                    continue;
                }
                tiles.push(Tile { tx: tx as u16, ty: ty as u16, data: pack_tile(read_tile(old, r), BPP) });
            }
        }
        TilePatchOf { tiles }
    }

    /// Memory held by the patch, in bytes.
    pub fn byte_len(&self) -> usize {
        self.tiles
            .iter()
            .map(|t| {
                std::mem::size_of::<Tile>()
                    + match &t.data {
                        TileData::Solid(_) => 0,
                        TileData::Raw(d) => d.len(),
                    }
            })
            .sum()
    }

    /// Pixel bounds of all tiles, clipped to a `width x height` buffer.
    pub fn bounds(&self, width: u32, height: u32) -> IRect {
        self.tiles.iter().fold(IRect::EMPTY, |acc, t| acc.union(&tile_rect(t.tx, t.ty, width, height)))
    }

    /// Swaps the patch's tiles with the same tiles of `buf`; returns the dirty bounds.
    ///
    /// A buffer of another kind (`B::BPP != BPP`) is refused: nothing changes and the result is `EMPTY`. Tiles that
    /// lie outside `buf` or whose size does not fit it are left alone (and not reported as dirty).
    pub fn swap_with<B: TileBuf>(&mut self, buf: &mut B) -> IRect {
        if !self.fits::<B>() {
            return IRect::EMPTY;
        }
        let [width, height] = buf.size();
        let mut dirty = IRect::EMPTY;
        for tile in &mut self.tiles {
            let r = tile_rect(tile.tx, tile.ty, width, height);
            if r.is_empty() {
                continue;
            }
            if let TileData::Raw(d) = &tile.data
                && d.len() != r.area() as usize * BPP
            {
                continue;
            }
            let previous = read_tile(buf, r);
            match &tile.data {
                TileData::Solid(c) => fill_tile(buf, r, *c),
                TileData::Raw(d) => write_tile(buf, r, d),
            }
            tile.data = pack_tile(previous, BPP);
            dirty = dirty.union(&r);
        }
        dirty
    }
}

/// Copy-on-first-touch backup of a buffer with `BPP` bytes per pixel during a stroke or fill.
pub struct TileBackupOf<const BPP: usize> {
    width: u32,
    height: u32,
    tiles: HashMap<(u16, u16), Box<[u8]>>,
}

/// The backup of a 4-byte buffer (paint layers, sticker pictures).
pub type TileBackup = TileBackupOf<4>;
/// The backup of a 1-byte buffer (sticker layer masks).
pub type MaskBackup = TileBackupOf<1>;

impl<const BPP: usize> TileBackupOf<BPP> {
    /// Empty backup for a `width x height` buffer.
    pub fn new(width: u32, height: u32) -> TileBackupOf<BPP> {
        TileBackupOf { width, height, tiles: HashMap::new() }
    }

    /// The backup's canvas clipped to `buf` (they are the same size unless the caller mixes buffers up).
    fn canvas<B: TileBuf>(&self, buf: &B) -> IRect {
        let [w, h] = buf.size();
        IRect::from_size(self.width.min(w), self.height.min(h))
    }

    /// Saves every not-yet-saved tile intersecting `r`; call BEFORE writing. A buffer of another kind
    /// (`B::BPP != BPP`) is ignored.
    pub fn touch_rect<B: TileBuf>(&mut self, buf: &B, r: IRect) {
        if B::BPP != BPP {
            debug_assert!(false, "tile backup of {BPP} bytes per pixel used on a buffer of {}", B::BPP);
            return;
        }
        let c = r.intersect(&self.canvas(buf));
        if c.is_empty() {
            return;
        }
        let tile = TILE as i32;
        let whole = buf_rect(buf);
        for ty in c.y0 / tile..=(c.y1 - 1) / tile {
            for tx in c.x0 / tile..=(c.x1 - 1) / tile {
                let key = (tx as u16, ty as u16);
                let (w, h) = (self.width, self.height);
                self.tiles.entry(key).or_insert_with(|| read_tile(buf, tile_rect(key.0, key.1, w, h).intersect(&whole)));
            }
        }
    }

    /// Undoes everything: writes the saved tiles back into `buf`; returns the dirty bounds (`EMPTY` for a buffer of
    /// another kind, which is left alone).
    pub fn restore<B: TileBuf>(&self, buf: &mut B) -> IRect {
        if B::BPP != BPP {
            return IRect::EMPTY;
        }
        let whole = buf_rect(buf);
        let mut dirty = IRect::EMPTY;
        for (&(tx, ty), data) in &self.tiles {
            let r = tile_rect(tx, ty, self.width, self.height).intersect(&whole);
            if r.is_empty() || data.len() != r.area() as usize * BPP {
                continue;
            }
            write_tile(buf, r, data);
            dirty = dirty.union(&r);
        }
        dirty
    }

    /// Converts the backup into an undo patch: drops tiles identical to `current`, collapses uniform tiles. A buffer
    /// of another kind gives an empty patch.
    pub fn into_patch<B: TileBuf>(self, current: &B) -> TilePatchOf<BPP> {
        if B::BPP != BPP {
            return TilePatchOf::empty();
        }
        let (width, height) = (self.width, self.height);
        let whole = buf_rect(current);
        // Sorted by (row, column): HashMap order must not leak into the patch (4.7).
        let mut saved: Vec<_> = self.tiles.into_iter().collect();
        saved.sort_unstable_by_key(|&((tx, ty), _)| (ty, tx));
        let mut tiles = Vec::new();
        for ((tx, ty), data) in saved {
            let r = tile_rect(tx, ty, width, height).intersect(&whole);
            if r.is_empty() || tile_equals(current, r, &data) {
                continue;
            }
            tiles.push(Tile { tx, ty, data: pack_tile(data, BPP) });
        }
        TilePatchOf { tiles }
    }

    /// True when no tile has been saved.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
}

impl TileBackupOf<4> {
    /// Pre-edit pixel (from the backup if saved, else from `buf`); for tests.
    pub fn original(&self, buf: &PixelBuf, x: u32, y: u32) -> [u8; 4] {
        let (tx, ty) = (x / TILE, y / TILE);
        if let Some(data) = self.tiles.get(&(tx as u16, ty as u16)) {
            let r = tile_rect(tx as u16, ty as u16, self.width, self.height);
            if r.contains(x as i32, y as i32) {
                let i = ((y - r.y0 as u32) as usize * r.width() as usize + (x - r.x0 as u32) as usize) * 4;
                if let Some(p) = data.get(i..i + 4) {
                    return [p[0], p[1], p[2], p[3]];
                }
            }
        }
        buf.get(x, y)
    }

    /// Pre-edit RGBA of the span x0..x1 of row y, appended to `out` after clearing it (4 * (x1 - x0) bytes): one tile
    /// lookup per 64 texels. This is what the stroke re-apply loop uses; never call `original` per texel there.
    ///
    /// Texels outside the layer (a caller bug) read as zero, so the length contract always holds.
    pub fn original_row(&self, buf: &PixelBuf, y: u32, x0: u32, x1: u32, out: &mut Vec<u8>) {
        self.original_span(buf, y, x0, x1, out);
    }
}

impl<const BPP: usize> TileBackupOf<BPP> {
    /// Pre-edit bytes of the span x0..x1 of row y (`BPP * (x1 - x0)` bytes, appended to `out` after clearing it), one
    /// tile lookup per 64 pixels; pixels outside the buffer read as zero. `original_row` for any kind of buffer.
    pub fn original_span<B: TileBuf>(&self, buf: &B, y: u32, x0: u32, x1: u32, out: &mut Vec<u8>) {
        out.clear();
        if x1 <= x0 {
            return;
        }
        out.reserve((x1 - x0) as usize * BPP);
        let canvas = self.canvas(buf);
        let row = y as usize * buf.size()[0] as usize * BPP;
        let ty = y / TILE;
        let mut x = x0;
        while x < x1 {
            let tx = x / TILE;
            let end = ((tx + 1) * TILE).min(x1);
            // The part of x..end that exists in the buffer.
            let valid_end = if (y as i32) < canvas.y1 && B::BPP == BPP { end.min(canvas.x1 as u32).max(x) } else { x };
            if valid_end > x {
                let n = (valid_end - x) as usize * BPP;
                let saved = self.tiles.get(&(tx as u16, ty as u16)).and_then(|data| {
                    let r = tile_rect(tx as u16, ty as u16, self.width, self.height);
                    let i = ((y - r.y0 as u32) as usize * r.width() as usize + (x - r.x0 as u32) as usize) * BPP;
                    data.get(i..i + n)
                });
                match saved {
                    Some(span) => out.extend_from_slice(span),
                    None => out.extend_from_slice(&buf.bytes()[row + x as usize * BPP..row + x as usize * BPP + n]),
                }
            }
            out.resize(out.len() + (end - valid_end) as usize * BPP, 0);
            x = end;
        }
    }
}

/// Pixel storage of one layer inside a structure snapshot.
#[derive(Clone)]
pub enum RecordPixels {
    /// Raster layer whose pixels the command does not replace: on restore the document's CURRENT buffer for this
    /// LayerId is reused (nothing is pinned).
    Keep,
    /// Pinned raster pixels (layer removed / merged / replaced by the command).
    Raster(Arc<PixelBuf>),
    /// Caches are always stored: they are replaced, never mutated in place.
    Stamp { stamp: StampId, cache: Arc<SparseBuf> },
    /// Caches are always stored: they are replaced, never mutated in place.
    Text { stamp: StampId, cache: Arc<SparseBuf> },
}

/// One layer inside a structure snapshot.
#[derive(Clone)]
pub struct LayerRecord {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub locked: bool,
    pub alpha_lock: bool,
    pub version: u64,
    pub pixels: RecordPixels,
    /// `Layer::content` at the time of the snapshot (restored with pinned raster pixels).
    pub content: Option<(u64, IRect)>,
}

/// The layer stack of one set inside a structure snapshot.
#[derive(Clone)]
pub struct SetStructure {
    pub layers: Vec<LayerRecord>,
    pub active: Option<LayerId>,
}

/// The complete layer/stamp structure of a document. Cheap: raster pixels are pinned only where a command asks for it.
#[derive(Clone)]
pub struct StructureSnapshot {
    pub sets: Vec<SetStructure>,
    pub stamps: Vec<StampObject>,
    /// Round 5 (5.20, 6.7): the stamps whose Full bake was still due (`Document::stamp_is_unbaked`). Their caches in
    /// `sets` are the earlier ones; restoring the snapshot restores "still needs its Full bake" with them.
    pub unbaked: BTreeSet<StampId>,
    /// Stickers (STICKER_PLAN 2.10): the document's sticker map (`Arc` clones; no pixel is copied).
    pub stickers: BTreeMap<StickerId, Arc<StickerDoc>>,
}

/// Payload of one history entry (swap semantics, section 6.7).
pub enum Edit {
    Pixels { set: TextureSetId, layer: LayerId, patch: TilePatch },
    Structure(Box<StructureSnapshot>),
    /// Stickers (STICKER_PLAN 2.10): a tile patch on an owned picture or a mask of one sticker layer. Applying it
    /// swaps the tiles through `Arc::make_mut` on the sticker and on the buffer (copy-on-write when a snapshot shares
    /// them), gives the buffer a new picture version and re-bakes the sticker's users once the whole entry is applied.
    StickerPixels { sticker: StickerId, layer: StickerLayerId, patch: StickerPatch },
    /// Applied in order for redo, reverse order for undo.
    Multi(Vec<Edit>),
}

/// One undo/redo step.
pub struct Entry {
    pub label: String,
    pub edit: Edit,
    /// History state id on the other side of this entry (swap semantics like `edit`): on the undo stack the state
    /// before it, on the redo stack the state after it (`History::state`, section 6.7).
    pub state: u64,
}

/// The open `Commit::Preview` edit: its label and the structure from before the first Preview call.
pub(crate) struct PendingEdit {
    pub label: String,
    pub before: StructureSnapshot,
}

/// Undo and redo stacks with a memory budget.
pub struct History {
    /// Default 512 MiB.
    pub budget_bytes: usize,
    /// Default 100.
    pub max_entries: usize,
    pub(crate) undo: Vec<Entry>,
    pub(crate) redo: Vec<Entry>,
    /// Id of the current history state (6.7): `Document::push_edit` gives every new state a fresh id (the revision
    /// after the push), undo and redo swap it with `Entry::state`. Equal ids mean equal document content.
    pub(crate) state: u64,
}

impl Default for History {
    /// 512 MiB, 100 entries, empty stacks, state 0.
    fn default() -> History {
        History { budget_bytes: 512 * 1024 * 1024, max_entries: 100, undo: Vec::new(), redo: Vec::new(), state: 0 }
    }
}

/// Address of a pixel buffer: the identity used to tell which buffers the document still holds.
fn buffer_key<T>(buf: &Arc<T>) -> usize {
    Arc::as_ptr(buf) as usize
}

/// The owned picture and mask buffers of a sticker map (`buffer_key`, bytes): what a snapshot keeps alive (2.7).
fn sticker_buffers(stickers: &BTreeMap<StickerId, Arc<StickerDoc>>) -> impl Iterator<Item = (usize, usize)> + '_ {
    stickers.values().flat_map(|doc| doc.layers.iter()).flat_map(|l| {
        let picture = l.owned_picture().map(|p| (buffer_key(p), p.byte_len()));
        let mask = l.mask.as_ref().map(|m| (buffer_key(&m.pixels), m.pixels.byte_len()));
        picture.into_iter().chain(mask)
    })
}

/// Bytes an edit keeps alive beyond the document: tile patches in full; of a structure snapshot the pinned raster
/// buffers, stamp/text caches and sticker pictures and masks that are neither in `live` (still used by the document)
/// nor in `seen` (already counted for a newer entry).
fn edit_cost(edit: &Edit, live: &HashSet<usize>, seen: &mut HashSet<usize>) -> usize {
    match edit {
        Edit::Pixels { patch, .. } => patch.byte_len(),
        Edit::StickerPixels { patch, .. } => patch.byte_len(),
        Edit::Structure(snapshot) => {
            let mut total = 0usize;
            for (key, bytes) in sticker_buffers(&snapshot.stickers) {
                if !live.contains(&key) && seen.insert(key) {
                    total = total.saturating_add(bytes);
                }
            }
            for record in snapshot.sets.iter().flat_map(|s| &s.layers) {
                let (key, bytes) = match &record.pixels {
                    RecordPixels::Keep => continue,
                    RecordPixels::Raster(buf) => (buffer_key(buf), buf.byte_len()),
                    RecordPixels::Stamp { cache, .. } | RecordPixels::Text { cache, .. } => (buffer_key(cache), cache.byte_len()),
                };
                if !live.contains(&key) && seen.insert(key) {
                    total = total.saturating_add(bytes);
                }
            }
            total
        }
        Edit::Multi(children) => children.iter().fold(0usize, |acc, c| acc.saturating_add(edit_cost(c, live, seen))),
    }
}

impl History {
    /// Pushes an entry: clears redo; evicts oldest entries over budget (newest always kept). The entry records the
    /// current `state` as its "before" state; the caller assigns the new state id (`Document::push_edit`).
    ///
    /// Without a document at hand every buffer pinned by a structure snapshot counts towards the budget; the
    /// document's own commands use the exact rule of section 6.7 (buffers the document still holds are free).
    pub fn push(&mut self, label: &str, edit: Edit) {
        self.push_with_live(label, edit, &HashSet::new());
    }

    /// `push` with the set of buffers (`buffer_key`) the document currently holds.
    fn push_with_live(&mut self, label: &str, edit: Edit, live: &HashSet<usize>) {
        self.redo.clear();
        self.undo.push(Entry { label: label.to_owned(), edit, state: self.state });
        self.evict(live);
    }

    /// Evicts the oldest undo entries while there are more than `max_entries` or their cost exceeds `budget_bytes`.
    fn evict(&mut self, live: &HashSet<usize>) {
        let max = self.max_entries.max(1);
        if self.undo.len() > max {
            let excess = self.undo.len() - max;
            self.undo.drain(..excess);
        }
        // Newest to oldest, charging a shared buffer to the newest entry that holds it (it is freed with that one).
        let mut seen = HashSet::new();
        let mut total = 0usize;
        let mut keep_from = 0;
        for i in (0..self.undo.len()).rev() {
            total = total.saturating_add(edit_cost(&self.undo[i].edit, live, &mut seen));
            if total > self.budget_bytes && i + 1 < self.undo.len() {
                keep_from = i + 1;
                break;
            }
        }
        self.undo.drain(..keep_from);
    }

    /// Bytes both stacks keep alive beyond the buffers in `live`.
    #[cfg(test)]
    fn cost(&self, live: &HashSet<usize>) -> usize {
        let mut seen = HashSet::new();
        self.undo
            .iter()
            .rev()
            .chain(self.redo.iter().rev())
            .fold(0usize, |acc, e| acc.saturating_add(edit_cost(&e.edit, live, &mut seen)))
    }

    /// Drops both stacks. The current state keeps its id; every other state becomes unreachable.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// Number of undoable entries.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Number of redoable entries.
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }
}

/// The snapshot record of a layer. Raster pixels are pinned only when `pin` is set; caches are always stored.
fn record_of(layer: &Layer, pin: bool) -> LayerRecord {
    let pixels = match &layer.kind {
        LayerKind::Raster(buf) if pin => RecordPixels::Raster(Arc::clone(buf)),
        LayerKind::Raster(_) => RecordPixels::Keep,
        LayerKind::Stamp { stamp, cache } => RecordPixels::Stamp { stamp: *stamp, cache: Arc::clone(cache) },
        LayerKind::Text { stamp, cache } => RecordPixels::Text { stamp: *stamp, cache: Arc::clone(cache) },
    };
    LayerRecord {
        id: layer.id,
        name: layer.name.clone(),
        visible: layer.visible,
        opacity: layer.opacity,
        blend: layer.blend,
        locked: layer.locked,
        alpha_lock: layer.alpha_lock,
        version: layer.version,
        pixels,
        content: layer.content,
    }
}

/// True when two states of a layer composite identically (same pixel storage, same compositing properties).
fn same_look(a: &Layer, b: &Layer) -> bool {
    a.visible == b.visible && a.opacity.to_bits() == b.opacity.to_bits() && a.blend == b.blend && a.same_pixels(b)
}

/// Area in which the composite of a set can differ after its layer list changed from `old` to `new`: the content
/// rects (`Layer::content_rects`) of all layers that were added, removed, moved or changed (section 6.7). Empty when
/// the lists are identical in ids, order, compositing properties and pixel storage.
fn structure_dirty(old: &[Layer], new: &[Layer]) -> Vec<IRect> {
    let old_index: HashMap<LayerId, usize> = old.iter().enumerate().map(|(i, l)| (l.id, i)).collect();
    let new_ids: HashSet<LayerId> = new.iter().map(|l| l.id).collect();
    let mut dirty = Vec::new();
    // Layers present in both lists, in their new order.
    let mut common_new: Vec<&Layer> = Vec::with_capacity(new.len());
    for layer in new {
        match old_index.get(&layer.id) {
            None => dirty.extend(layer.content_rects()),
            Some(&i) => {
                if !same_look(&old[i], layer) {
                    dirty.extend(old[i].content_rects());
                    dirty.extend(layer.content_rects());
                }
                common_new.push(layer);
            }
        }
    }
    let mut common_old = Vec::with_capacity(common_new.len());
    for layer in old {
        if new_ids.contains(&layer.id) {
            common_old.push(layer);
        } else {
            dirty.extend(layer.content_rects());
        }
    }
    // Reordered layers: every position at which the two orders disagree.
    for (a, b) in common_old.iter().zip(&common_new) {
        if a.id != b.id {
            dirty.extend(a.content_rects());
            dirty.extend(b.content_rects());
        }
    }
    dirty
}

impl Document {
    /// Undoes the newest entry and returns its label; None if nothing to undo (or a stroke is open).
    ///
    /// A pending Preview edit is flushed first, so it is the entry that gets undone.
    pub fn undo(&mut self) -> Option<String> {
        if self.any_stroke() {
            return None;
        }
        self.flush_pending_edit();
        let mut entry = self.history.undo.pop()?;
        self.apply_edit(&mut entry.edit, true);
        self.settle_sticker_pixel_edits();
        std::mem::swap(&mut entry.state, &mut self.history.state);
        let label = entry.label.clone();
        self.history.redo.push(entry);
        self.touch();
        Some(label)
    }

    /// Redoes the last undone entry and returns its label; None if nothing to redo (or a stroke is open).
    ///
    /// A pending Preview edit is flushed first; like every new entry it clears the redo stack.
    pub fn redo(&mut self) -> Option<String> {
        if self.any_stroke() {
            return None;
        }
        self.flush_pending_edit();
        let mut entry = self.history.redo.pop()?;
        self.apply_edit(&mut entry.edit, false);
        self.settle_sticker_pixel_edits();
        std::mem::swap(&mut entry.state, &mut self.history.state);
        let label = entry.label.clone();
        self.history.undo.push(entry);
        self.touch();
        Some(label)
    }

    /// Exchanges the payload of `edit` with the document (swap semantics, section 6.7).
    fn apply_edit(&mut self, edit: &mut Edit, undo: bool) {
        match edit {
            Edit::Pixels { layer, patch, .. } => {
                let id = *layer;
                let Some(LayerKind::Raster(buf)) = self.layer_mut(id).map(|l| &mut l.kind) else {
                    debug_assert!(false, "history: pixel edit for {id:?}, which is not an existing paint layer");
                    return;
                };
                let dirty = patch.swap_with(Arc::make_mut(buf));
                self.mark_layer_dirty(id, dirty);
            }
            Edit::StickerPixels { sticker, layer, patch } => self.swap_sticker_patch(*sticker, *layer, patch),
            Edit::Structure(snapshot) => {
                let empty =
                    StructureSnapshot { sets: Vec::new(), stamps: Vec::new(), unbaked: BTreeSet::new(), stickers: BTreeMap::new() };
                let stored = std::mem::replace(&mut **snapshot, empty);
                **snapshot = self.restore_structure(stored);
            }
            Edit::Multi(children) => {
                if undo {
                    children.iter_mut().rev().for_each(|c| self.apply_edit(c, true));
                } else {
                    children.iter_mut().for_each(|c| self.apply_edit(c, false));
                }
            }
        }
    }

    /// True when `undo` would do something.
    pub fn can_undo(&self) -> bool {
        !self.any_stroke() && (self.pending_edit.is_some() || !self.history.undo.is_empty())
    }

    /// True when `redo` would do something.
    pub fn can_redo(&self) -> bool {
        !self.any_stroke() && self.pending_edit.is_none() && !self.history.redo.is_empty()
    }

    /// Label of the entry `undo` would undo.
    pub fn undo_label(&self) -> Option<&str> {
        if !self.can_undo() {
            return None;
        }
        match &self.pending_edit {
            Some(pending) => Some(&pending.label),
            None => self.history.undo.last().map(|e| e.label.as_str()),
        }
    }

    /// Label of the entry `redo` would redo.
    pub fn redo_label(&self) -> Option<&str> {
        if !self.can_redo() {
            return None;
        }
        self.history.redo.last().map(|e| e.label.as_str())
    }

    /// True while a `Commit::Preview` edit is open.
    pub fn has_pending_edit(&self) -> bool {
        self.pending_edit.is_some()
    }

    /// Ends the pending Preview edit: first `finalize_stamp_bake(id)` (5.20) for every id drained from `preview_baked`
    /// (ascending), then pushes `Edit::Structure(before)` as one history entry. No-op without a pending edit.
    ///
    /// While `finalize_stamp_bake(id)` runs, the edit is still pending and `id` is still listed in `preview_baked`;
    /// the set is empty before the entry is pushed. With deferred Final bakes on (round 5) `finalize_stamp_bake` bakes
    /// nothing: the stamps are left unbaked (`stamp_is_unbaked`) and the entry is pushed at once.
    pub fn flush_pending_edit(&mut self) {
        if self.pending_edit.is_none() {
            return;
        }
        let ids: Vec<StampId> = self.preview_baked.iter().copied().collect();
        for id in ids {
            self.finalize_stamp_bake(id);
        }
        self.preview_baked.clear();
        if let Some(pending) = self.pending_edit.take() {
            self.push_structure_edit(&pending.label, pending.before);
        }
    }

    /// Restores the state from before the first Preview call (Full-quality caches included) and clears `preview_baked`.
    /// The unbaked set comes back with it (round 5): a stamp whose Full bake was due before the edit is due again.
    ///
    /// Returns false when no edit was pending. Pushes no history entry and does not bump the revision.
    pub fn cancel_pending_edit(&mut self) -> bool {
        let Some(pending) = self.pending_edit.take() else {
            return false;
        };
        self.preview_baked.clear();
        self.bakes.preview_stale.clear();
        self.restore_structure(pending.before);
        true
    }

    /// Snapshot of the current structure. Raster layers listed in `pin` are stored as `RecordPixels::Raster` (Arc clone),
    /// every other raster layer as `Keep`; stamp/text caches are always stored.
    pub(crate) fn snapshot_structure(&self, pin: &[LayerId]) -> StructureSnapshot {
        StructureSnapshot {
            sets: self
                .sets
                .iter()
                .map(|set| SetStructure {
                    layers: set.layers.iter().map(|l| record_of(l, pin.contains(&l.id))).collect(),
                    active: set.active,
                })
                .collect(),
            stamps: self.stamps.clone(),
            unbaked: self.bakes.unbaked.clone(),
            stickers: self.sticker_state.map.clone(),
        }
    }

    /// Swaps the document's structure with `s` (rules in 6.7), marks dirty, returns the inverse snapshot. The unbaked
    /// set is swapped too (round 5): ids of stamps that do not exist after the restore are dropped; with deferred
    /// Final bakes off, a restored unbaked stamp is baked here (a snapshot from while the mode was on).
    pub(crate) fn restore_structure(&mut self, s: StructureSnapshot) -> StructureSnapshot {
        debug_assert_eq!(s.sets.len(), self.sets.len(), "structure snapshot of a different document");

        // What the document looks like now. A raster layer that `s` keeps is not pinned; one that is about to
        // disappear or to be replaced is.
        let kept: HashSet<LayerId> =
            s.sets.iter().flat_map(|set| &set.layers).filter(|r| matches!(r.pixels, RecordPixels::Keep)).map(|r| r.id).collect();
        let inverse = StructureSnapshot {
            sets: self
                .sets
                .iter()
                .map(|set| SetStructure {
                    layers: set.layers.iter().map(|l| record_of(l, !kept.contains(&l.id))).collect(),
                    active: set.active,
                })
                .collect(),
            stamps: self.stamps.clone(),
            unbaked: self.bakes.unbaked.clone(),
            stickers: self.sticker_state.map.clone(),
        };

        // Buffers, versions and content bounds the document currently holds, for the `Keep` records.
        type Current = (Arc<PixelBuf>, u64, Option<(u64, IRect)>);
        let current: HashMap<LayerId, Current> = self
            .sets
            .iter()
            .flat_map(|set| &set.layers)
            .filter_map(|l| match &l.kind {
                LayerKind::Raster(buf) => Some((l.id, (Arc::clone(buf), l.version, l.content))),
                _ => None,
            })
            .collect();

        let StructureSnapshot { sets: new_sets, stamps: new_stamps, unbaked: new_unbaked, stickers: new_stickers } = s;
        let mut version_counter = self.version_counter;
        for (set, structure) in self.sets.iter_mut().zip(new_sets) {
            let layers: Vec<Layer> = structure
                .layers
                .into_iter()
                .map(|r| {
                    let (kind, version, content) = match r.pixels {
                        RecordPixels::Keep => match current.get(&r.id) {
                            Some((buf, version, content)) => (LayerKind::Raster(Arc::clone(buf)), *version, *content),
                            None => {
                                debug_assert!(false, "history: no current paint layer {:?} for a Keep record", r.id);
                                version_counter += 1;
                                (LayerKind::empty_raster(), version_counter, None)
                            }
                        },
                        RecordPixels::Raster(buf) => (LayerKind::Raster(buf), r.version, r.content),
                        RecordPixels::Stamp { stamp, cache } => (LayerKind::Stamp { stamp, cache }, r.version, None),
                        RecordPixels::Text { stamp, cache } => (LayerKind::Text { stamp, cache }, r.version, None),
                    };
                    Layer {
                        id: r.id,
                        name: r.name,
                        visible: r.visible,
                        opacity: r.opacity,
                        blend: r.blend,
                        locked: r.locked,
                        alpha_lock: r.alpha_lock,
                        kind,
                        version,
                        content,
                    }
                })
                .collect();
            let old = std::mem::replace(&mut set.layers, layers);
            set.active = structure.active;
            let dirty = structure_dirty(&old, &set.layers);
            set.mark_rects_dirty(&dirty);
        }
        self.version_counter = version_counter;
        drop(current);
        self.stamps = new_stamps;
        // Stickers (2.10): the sticker map comes back with the stamps that show it; the linked caches in `sets` are the
        // ones baked from it, so no bake is needed. Derived pictures of changed stickers are found stale by their `Arc`.
        self.sticker_state.map = new_stickers;
        self.prune_sticker_caches();
        let stamps = &self.stamps;
        self.bakes.unbaked = new_unbaked.into_iter().filter(|id| stamps.iter().any(|s| s.id == *id)).collect();

        // Rasterized text of stamps that no longer exist, or whose text changed, is stale.
        let stamps = &self.stamps;
        self.text_mips.retain(|id, (cached, _)| {
            stamps.iter().any(|s| s.id == *id && matches!(&s.source, StampSource::Text(spec) if *spec == *cached))
        });
        if !self.defer_final_bakes && !self.bakes.unbaked.is_empty() {
            self.settle_final_bakes();
        }

        self.debug_check_structure();
        inverse
    }

    /// Addresses of every pixel buffer the document's layers and stickers hold right now.
    fn live_buffers(&self) -> HashSet<usize> {
        self.sets
            .iter()
            .flat_map(|set| &set.layers)
            .map(|l| match &l.kind {
                LayerKind::Raster(buf) => buffer_key(buf),
                LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => buffer_key(cache),
            })
            .chain(sticker_buffers(&self.sticker_state.map).map(|(key, _)| key))
            .collect()
    }

    /// Bytes the history keeps alive beyond the document's own buffers (the quantity the budget limits).
    #[cfg(test)]
    pub(crate) fn history_bytes(&self) -> usize {
        self.history.cost(&self.live_buffers())
    }

    /// Runs `f`, several commands, as one undo step (fix_export item 5: Delete and Duplicate on a multi-selection):
    /// the history entries the commands push are merged into one entry named `label` (`Edit::Multi` in push order;
    /// one entry is only renamed, none stays none). Its "before" state is the state before the first command, so one
    /// undo restores the document as it was and one redo repeats every command. The budget and the entry limit are
    /// applied once at the end (not in between, where they could evict entries of the group).
    pub(crate) fn group_edits<R>(&mut self, label: &str, f: impl FnOnce(&mut Document) -> R) -> R {
        self.flush_pending_edit();
        let (max, budget) = (self.history.max_entries, self.history.budget_bytes);
        self.history.max_entries = usize::MAX;
        self.history.budget_bytes = usize::MAX;
        let start = self.history.undo.len();
        let result = f(self);
        self.history.max_entries = max;
        self.history.budget_bytes = budget;
        let mut added = self.history.undo.split_off(start.min(self.history.undo.len()));
        if added.len() > 1 {
            let state = added[0].state;
            let edits: Vec<Edit> = added.into_iter().map(|e| e.edit).collect();
            self.history.undo.push(Entry { label: label.to_owned(), edit: Edit::Multi(edits), state });
        } else if let Some(mut entry) = added.pop() {
            entry.label = label.to_owned();
            self.history.undo.push(entry);
        }
        let live = self.live_buffers();
        self.history.evict(&live);
        result
    }

    /// `history.push` + `touch()` + `debug_check_structure`.
    pub(crate) fn push_structure_edit(&mut self, label: &str, before: StructureSnapshot) {
        self.push_edit(label, Edit::Structure(Box::new(before)));
        self.debug_check_structure();
    }

    /// No-op if a pending edit with the same label exists; else `flush_pending_edit()` + capture `snapshot_structure(&[])`.
    pub(crate) fn begin_pending(&mut self, label: &str) {
        if self.pending_edit.as_ref().is_some_and(|p| p.label == label) {
            return;
        }
        self.flush_pending_edit();
        self.pending_edit = Some(Box::new(PendingEdit { label: label.to_owned(), before: self.snapshot_structure(&[]) }));
    }

    /// `history.push` + `touch()`; the new history state gets the new revision as its id (fresh: the revision only
    /// grows).
    pub(crate) fn push_edit(&mut self, label: &str, edit: Edit) {
        let live = self.live_buffers();
        self.history.push_with_live(label, edit, &live);
        self.touch();
        self.history.state = self.revision;
    }
}

/// Helpers shared by the tests of `history.rs`, `doc_layers.rs` and `doc_build.rs`.
#[cfg(test)]
pub(crate) mod testkit {
    use glam::Vec2;

    use super::*;
    use crate::blob::BlobStore;
    use crate::composite::flatten_layers;
    use crate::doc_build::{DocumentParts, SetParts};
    use crate::ids::BlobId;
    use crate::math::fnv1a64;
    use crate::progress::NoProgress;
    use crate::samples;
    use crate::stamp::{Placement, UvPlacement};
    use crate::testutil::imported_from_raw;

    /// Bytes compared exactly but printed as a length and a hash.
    #[derive(Clone, PartialEq)]
    pub(crate) struct Bytes(pub Vec<u8>);

    impl std::fmt::Debug for Bytes {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Bytes(len {}, hash {:016x})", self.0.len(), fnv1a64(0xcbf2_9ce4_8422_2325, &self.0))
        }
    }

    #[derive(Clone, PartialEq, Debug)]
    pub(crate) struct LayerState {
        pub id: LayerId,
        pub name: String,
        pub visible: bool,
        pub opacity_bits: u32,
        pub blend: BlendMode,
        pub locked: bool,
        pub alpha_lock: bool,
        pub kind: &'static str,
        pub stamp: Option<StampId>,
        pub rect: IRect,
        pub pixels: Bytes,
    }

    #[derive(Clone, PartialEq, Debug)]
    pub(crate) struct SetState {
        pub active: Option<LayerId>,
        pub layers: Vec<LayerState>,
        pub composite: Bytes,
    }

    /// Everything undo / redo must reproduce: structure, properties, pixels, stamps and composites.
    #[derive(Clone, PartialEq, Debug)]
    pub(crate) struct DocState {
        pub sets: Vec<SetState>,
        pub stamps: Vec<StampObject>,
    }

    /// Captures the document after bringing the composites up to date. Also checks that the dirty tracking was
    /// complete (composite == flatten) and that the structure invariants hold.
    pub(crate) fn doc_state(doc: &mut Document) -> DocState {
        doc.recomposite_all();
        doc.debug_check_structure();
        let sets = doc
            .sets
            .iter()
            .map(|set| {
                assert!(
                    set.composite == flatten_layers(&set.layers, set.width, set.height, false),
                    "set {:?}: the composite is stale (a change was not marked dirty)",
                    set.id
                );
                SetState {
                    active: set.active,
                    layers: set
                        .layers
                        .iter()
                        .map(|l| LayerState {
                            id: l.id,
                            name: l.name.clone(),
                            visible: l.visible,
                            opacity_bits: l.opacity.to_bits(),
                            blend: l.blend,
                            locked: l.locked,
                            alpha_lock: l.alpha_lock,
                            kind: l.kind_name(),
                            stamp: l.stamp_id(),
                            // Paint layers compare by pixels (an unallocated layer equals an all-zero one), caches
                            // by their parts too.
                            rect: if l.is_raster() { IRect::EMPTY } else { l.content_rect() },
                            pixels: Bytes(l.to_dense(set.width, set.height).data),
                        })
                        .collect(),
                    composite: Bytes(set.composite.data.clone()),
                }
            })
            .collect();
        DocState { sets, stamps: doc.stamps.clone() }
    }

    /// A raw pixel edit the way the brush engine does it: tile backup, write, `mark_layer_dirty`, `push_edit`.
    pub(crate) fn paint_rect(doc: &mut Document, layer: LayerId, r: IRect, rgba: [u8; 4]) {
        let (set, index) = doc.find_layer(layer).expect("paint_rect: unknown layer");
        let target = &mut doc.sets[set.index()];
        target.allocate_raster(index);
        let mut backup = TileBackup::new(target.width, target.height);
        let LayerKind::Raster(buf) = &mut target.layers[index].kind else { panic!("paint_rect: not a paint layer") };
        backup.touch_rect(buf, r);
        let pixels = Arc::make_mut(buf);
        let c = r.intersect(&pixels.rect());
        for y in c.y0..c.y1 {
            for x in c.x0..c.x1 {
                pixels.set(x as u32, y as u32, rgba);
            }
        }
        let patch = backup.into_patch(pixels);
        doc.mark_layer_dirty(layer, r);
        doc.push_edit("Paint", Edit::Pixels { set, layer, patch });
    }

    /// The pixel `Arc` of a paint layer.
    pub(crate) fn raster_arc(doc: &Document, layer: LayerId) -> &Arc<PixelBuf> {
        match &doc.layer(layer).expect("unknown layer").kind {
            LayerKind::Raster(buf) => buf,
            _ => panic!("not a paint layer"),
        }
    }

    /// How each record of the newest undo entry stores its pixels, per set ("keep" / "raster" / "stamp" / "text").
    pub(crate) fn record_kinds(snapshot: &StructureSnapshot) -> Vec<Vec<(LayerId, &'static str)>> {
        snapshot
            .sets
            .iter()
            .map(|s| {
                s.layers
                    .iter()
                    .map(|r| {
                        let kind = match r.pixels {
                            RecordPixels::Keep => "keep",
                            RecordPixels::Raster(_) => "raster",
                            RecordPixels::Stamp { .. } => "stamp",
                            RecordPixels::Text { .. } => "text",
                        };
                        (r.id, kind)
                    })
                    .collect()
            })
            .collect()
    }

    pub(crate) const STAMP: StampId = StampId(100);
    pub(crate) const SIZE: u32 = 96;

    fn cache(rect: IRect, seed: u8) -> Arc<SparseBuf> {
        let mut c = crate::pixel::CroppedBuf::new(rect);
        for (i, px) in c.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *px = [seed, (i * 7) as u8, (i * 13) as u8, 64 + (i * 5 % 192) as u8];
        }
        Arc::new(SparseBuf::from_cropped(c))
    }

    /// Two 96 x 96 sets built with `from_parts`:
    /// set 0 = [paint 1 (white), stamp layer 101, paint 3 (transparent, active)],
    /// set 1 = [paint 2 (white, active), stamp layer 102].
    /// Stamp 100 (image, UV placement) is linked to 101 and 102; its home set is 0. `next_id` is 200.
    pub(crate) fn doc_with_stamp() -> Document {
        let model = imported_from_raw(samples::two_material_model(), Vec::new());
        assert_eq!(model.mesh.sets.len(), 2);
        let white = || LayerKind::Raster(Arc::new(PixelBuf::filled(SIZE, SIZE, [255; 4])));
        let stamp_layer = |id: u64, rect: IRect| {
            Layer::new(LayerId(id), "Decal", LayerKind::Stamp { stamp: STAMP, cache: cache(rect, id as u8) })
        };
        let sets = vec![
            SetParts {
                name: model.mesh.sets[0].name.clone(),
                width: SIZE,
                height: SIZE,
                layers: vec![
                    Layer::new(LayerId(1), "Layer 1", white()),
                    stamp_layer(101, IRect::new(10, 12, 50, 40)),
                    Layer::new(LayerId(3), "Paint 1", LayerKind::Raster(Arc::new(PixelBuf::new(SIZE, SIZE)))),
                ],
                active: Some(LayerId(3)),
            },
            SetParts {
                name: model.mesh.sets[1].name.clone(),
                width: SIZE,
                height: SIZE,
                layers: vec![Layer::new(LayerId(2), "Layer 1", white()), stamp_layer(102, IRect::new(60, 5, 90, 30))],
                active: Some(LayerId(2)),
            },
        ];
        let stamp = StampObject {
            id: STAMP,
            name: "Decal".to_owned(),
            source: StampSource::Image { blob: BlobId(0) },
            placement: Placement::Uv(UvPlacement {
                set: TextureSetId(0),
                center_px: Vec2::new(30.0, 26.0),
                size_px: Vec2::new(40.0, 28.0),
                rotation: 0.0,
                flip_x: false,
                flip_y: false,
                clip_to_islands: false,
            }),
            tint: [255; 4],
            home_set: TextureSetId(0),
            styles: Default::default(),
        };
        let parts = DocumentParts {
            name: "stamped".to_owned(),
            model,
            sets,
            stamps: vec![stamp],
            blobs: BlobStore::default(),
            ui_state: serde_json::Value::Null,
            next_id: 200,
        };
        Document::from_parts(parts, &NoProgress).expect("doc_with_stamp")
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use crate::samples;
    use crate::testutil::{XorShift, doc_from_raw};

    fn random_buf(rng: &mut XorShift, w: u32, h: u32) -> PixelBuf {
        let mut buf = PixelBuf::new(w, h);
        for b in &mut buf.data {
            *b = (rng.next_u32() & 0xff) as u8;
        }
        buf
    }

    fn fill(buf: &mut PixelBuf, r: IRect, rgba: [u8; 4]) {
        let c = r.intersect(&buf.rect());
        for y in c.y0..c.y1 {
            for x in c.x0..c.x1 {
                buf.set(x as u32, y as u32, rgba);
            }
        }
    }

    fn raw_edit(bytes: usize) -> Edit {
        let data = TileData::Raw(vec![1u8; bytes].into_boxed_slice());
        Edit::Pixels { set: TextureSetId(0), layer: LayerId(1), patch: TilePatch { tiles: vec![Tile { tx: 0, ty: 0, data }] } }
    }

    fn pinned_edit(buf: &Arc<PixelBuf>) -> Edit {
        let record = LayerRecord {
            id: LayerId(1),
            name: "x".to_owned(),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            locked: false,
            alpha_lock: false,
            version: 1,
            pixels: RecordPixels::Raster(Arc::clone(buf)),
            content: None,
        };
        let sets = vec![SetStructure { layers: vec![record], active: None }];
        Edit::Structure(Box::new(StructureSnapshot { sets, stamps: Vec::new(), unbaked: BTreeSet::new(), stickers: BTreeMap::new() }))
    }

    #[test]
    fn tile_patch_swap_is_its_own_inverse() {
        let mut rng = XorShift::new(1);
        // 150 x 100: the right and bottom tiles are 22 and 36 pixels.
        let original = random_buf(&mut rng, 150, 100);
        let mut buf = original.clone();
        let mut backup = TileBackup::new(150, 100);
        assert!(backup.is_empty());
        let rects =
            [IRect::new(5, 5, 20, 20), IRect::new(120, 60, 150, 100), IRect::new(60, 30, 70, 70), IRect::new(-10, 90, 3, 200)];
        for (i, r) in rects.iter().enumerate() {
            backup.touch_rect(&buf, *r);
            fill(&mut buf, *r, [i as u8, 200, 100, 255]);
        }
        assert!(!backup.is_empty());
        let edited = buf.clone();
        let mut patch = backup.into_patch(&buf);
        // (0,0) from the first rect; (1,0) and (1,1) from the third; (1,0)..(2,1) from the second; (0,1) from the fourth.
        let tiles: Vec<(u16, u16)> = patch.tiles.iter().map(|t| (t.tx, t.ty)).collect();
        assert_eq!(tiles, [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)], "sorted by row, then column");
        assert_eq!(patch.bounds(150, 100), IRect::new(0, 0, 150, 100));
        assert!(!patch.is_empty());
        // Edge tiles hold only the pixels that exist.
        let edge = patch.tiles.iter().find(|t| (t.tx, t.ty) == (2, 1)).unwrap();
        match &edge.data {
            TileData::Raw(d) => assert_eq!(d.len(), 22 * 36 * 4),
            TileData::Solid(_) => panic!("random pixels cannot be solid"),
        }

        let dirty = patch.swap_with(&mut buf);
        assert_eq!(buf, original);
        assert_eq!(dirty, IRect::new(0, 0, 150, 100));
        patch.swap_with(&mut buf);
        assert_eq!(buf, edited);
        patch.swap_with(&mut buf);
        assert_eq!(buf, original);

        // A patch applied to a buffer of another size leaves raw tiles that do not fit alone and never writes outside.
        let mut small = PixelBuf::new(100, 40);
        let dirty = patch.swap_with(&mut small);
        assert!(dirty.y1 <= 40 && dirty.x1 <= 100, "{dirty:?}");
    }

    #[test]
    fn solid_tiles_compress() {
        let mut buf = PixelBuf::filled(256, 192, [9, 9, 9, 255]);
        let mut backup = TileBackup::new(256, 192);
        let r = IRect::new(0, 0, 256, 192);
        backup.touch_rect(&buf, r);
        fill(&mut buf, r, [200, 0, 0, 255]);
        let mut patch = backup.into_patch(&buf);
        assert_eq!(patch.tiles.len(), 12);
        assert!(patch.tiles.iter().all(|t| matches!(t.data, TileData::Solid([9, 9, 9, 255]))));
        assert_eq!(patch.byte_len(), 12 * std::mem::size_of::<Tile>());
        assert!(patch.byte_len() < 1024, "{} bytes for 196608 bytes of pixels", patch.byte_len());

        // After the swap the patch holds the (equally uniform) fill and stays compressed.
        patch.swap_with(&mut buf);
        assert_eq!(buf, PixelBuf::filled(256, 192, [9, 9, 9, 255]));
        assert!(patch.tiles.iter().all(|t| matches!(t.data, TileData::Solid([200, 0, 0, 255]))));

        // A non-uniform tile is stored raw and costs its pixels.
        let mut backup = TileBackup::new(256, 192);
        backup.touch_rect(&buf, IRect::new(0, 0, 1, 1));
        buf.set(0, 0, [1, 2, 3, 4]);
        buf.set(1, 0, [5, 6, 7, 8]);
        let mut patch = backup.into_patch(&buf);
        assert!(matches!(patch.tiles[0].data, TileData::Solid(_)), "the pre-edit tile was uniform");
        patch.swap_with(&mut buf);
        assert!(matches!(&patch.tiles[0].data, TileData::Raw(d) if d.len() == 64 * 64 * 4));
        assert_eq!(patch.byte_len(), std::mem::size_of::<Tile>() + 64 * 64 * 4);
    }

    #[test]
    fn into_patch_drops_unchanged_tiles() {
        let mut rng = XorShift::new(2);
        let original = random_buf(&mut rng, 200, 200);
        let mut buf = original.clone();
        let mut backup = TileBackup::new(200, 200);
        // Touch all 16 tiles, change a pixel in one of them.
        backup.touch_rect(&buf, IRect::new(0, 0, 200, 200));
        let p = buf.get(70, 130);
        buf.set(70, 130, [p[0] ^ 1, p[1], p[2], p[3]]);
        let patch = backup.into_patch(&buf);
        assert_eq!(patch.tiles.len(), 1);
        assert_eq!((patch.tiles[0].tx, patch.tiles[0].ty), (1, 2));
        assert_eq!(patch.bounds(200, 200), IRect::new(64, 128, 128, 192));

        // Touched but not changed at all: an empty patch.
        let mut backup = TileBackup::new(200, 200);
        backup.touch_rect(&buf, IRect::new(10, 10, 190, 190));
        let patch = backup.into_patch(&buf);
        assert!(patch.is_empty());
        assert_eq!(patch.byte_len(), 0);
        assert_eq!(patch.bounds(200, 200), IRect::EMPTY);

        // Nothing touched, or only rects outside the layer: nothing saved.
        let mut backup = TileBackup::new(200, 200);
        backup.touch_rect(&buf, IRect::EMPTY);
        backup.touch_rect(&buf, IRect::new(200, 0, 300, 50));
        backup.touch_rect(&buf, IRect::new(-50, -50, 0, 0));
        assert!(backup.is_empty());
    }

    #[test]
    fn backup_keeps_the_first_version_and_restores_it() {
        let mut rng = XorShift::new(3);
        let original = random_buf(&mut rng, 130, 70);
        let mut buf = original.clone();
        let mut backup = TileBackup::new(130, 70);
        // The same tiles are touched twice; the second touch must not overwrite the saved original.
        for round in 0..2u8 {
            let r = IRect::new(100, 40, 130, 70);
            backup.touch_rect(&buf, r);
            fill(&mut buf, r, [round, round, round, 255]);
        }
        backup.touch_rect(&buf, IRect::new(0, 0, 10, 10));
        fill(&mut buf, IRect::new(0, 0, 10, 10), [7; 4]);
        for (x, y) in [(0, 0), (9, 9), (63, 63), (64, 0), (100, 40), (129, 69), (128, 64), (70, 10)] {
            assert_eq!(backup.original(&buf, x, y), original.get(x, y), "({x}, {y})");
        }
        let dirty = backup.restore(&mut buf);
        assert_eq!(buf, original);
        // Saved tiles: (0,0), and (1,0), (2,0), (1,1), (2,1) around the lower right rect.
        assert_eq!(dirty, IRect::new(0, 0, 130, 70));
        assert_eq!(TileBackup::new(130, 70).restore(&mut buf), IRect::EMPTY);
    }

    #[test]
    fn original_row_equals_original_texel_by_texel() {
        let mut rng = XorShift::new(4);
        let (w, h) = (150u32, 100u32);
        let original = random_buf(&mut rng, w, h);
        let mut buf = original.clone();
        let mut backup = TileBackup::new(w, h);
        for _ in 0..6 {
            let x0 = (rng.next_u32() % w) as i32;
            let y0 = (rng.next_u32() % h) as i32;
            let r = IRect::new(x0, y0, x0 + 1 + (rng.next_u32() % 40) as i32, y0 + 1 + (rng.next_u32() % 40) as i32);
            backup.touch_rect(&buf, r);
            fill(&mut buf, r, [1, 2, 3, 4]);
        }
        let mut out = vec![0xaa; 3];
        for y in 0..h {
            for (x0, x1) in [(0, w), (0, 1), (63, 65), (64, 128), (1, 149), (140, 150), (77, 77), (90, 80)] {
                backup.original_row(&buf, y, x0, x1, &mut out);
                assert_eq!(out.len(), 4 * x1.saturating_sub(x0) as usize, "row {y}, span {x0}..{x1}");
                for (i, px) in out.as_chunks::<4>().0.iter().enumerate() {
                    let x = x0 + i as u32;
                    assert_eq!(*px, backup.original(&buf, x, y), "({x}, {y})");
                    assert_eq!(*px, original.get(x, y), "({x}, {y})");
                }
            }
        }
        // Outside the layer the span is zero-filled, so the length contract holds.
        backup.original_row(&buf, 5, 140, 160, &mut out);
        assert_eq!(out.len(), 80);
        assert_eq!(out[..40], original.row(5)[560..]);
        assert!(out[40..].iter().all(|&b| b == 0));
        backup.original_row(&buf, h + 3, 0, 8, &mut out);
        assert_eq!(out, vec![0; 32]);
    }

    #[test]
    fn budget_eviction_keeps_the_newest_entry() {
        let tile = std::mem::size_of::<Tile>();
        let mut history = History { budget_bytes: 10_000, ..History::default() };
        assert_eq!((history.max_entries, History::default().budget_bytes), (100, 512 * 1024 * 1024));
        for i in 0..3 {
            history.push(&format!("e{i}"), raw_edit(3000));
        }
        assert_eq!(history.undo_len(), 3);
        assert_eq!(history.cost(&HashSet::new()), 3 * (3000 + tile));
        history.push("e3", raw_edit(3000));
        assert_eq!(history.undo_len(), 3, "the oldest entry is evicted when the budget is exceeded");
        assert_eq!(history.undo[0].label, "e1");

        // An entry larger than the whole budget evicts everything else but stays itself.
        history.push("huge", raw_edit(50_000));
        assert_eq!(history.undo_len(), 1);
        assert_eq!(history.undo[0].label, "huge");
        history.push("small", raw_edit(10));
        assert_eq!(history.undo_len(), 1);
        assert_eq!(history.undo[0].label, "small");

        // Entry count limit.
        let mut history = History { max_entries: 3, ..History::default() };
        for i in 0..5 {
            history.push(&format!("e{i}"), raw_edit(8));
        }
        let labels: Vec<&str> = history.undo.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["e2", "e3", "e4"]);
        // max_entries == 0 still keeps the newest.
        let mut history = History { max_entries: 0, ..History::default() };
        history.push("a", raw_edit(8));
        history.push("b", raw_edit(8));
        assert_eq!(history.undo_len(), 1);
        assert_eq!(history.undo[0].label, "b");

        history.clear();
        assert_eq!((history.undo_len(), history.redo_len()), (0, 0));
    }

    #[test]
    fn pinned_buffers_are_charged_once_and_only_when_the_document_dropped_them() {
        let a = Arc::new(PixelBuf::new(32, 32)); // 4096 bytes
        let b = Arc::new(PixelBuf::new(32, 32));
        let mut history = History { budget_bytes: 5000, ..History::default() };
        history.push("a1", pinned_edit(&a));
        history.push("a2", Edit::Multi(vec![pinned_edit(&a), raw_edit(100)]));
        assert_eq!(history.undo_len(), 2, "the same buffer in two entries is counted once");
        assert_eq!(history.cost(&HashSet::new()), 4096 + 100 + std::mem::size_of::<Tile>());
        // A buffer the document still holds is free.
        let live: HashSet<usize> = [buffer_key(&a)].into_iter().collect();
        assert_eq!(history.cost(&live), 100 + std::mem::size_of::<Tile>());
        history.push_with_live("b", pinned_edit(&b), &[buffer_key(&b)].into_iter().collect());
        assert_eq!(history.undo_len(), 3, "b is live, so only a is charged");
        // Without the document's knowledge both buffers count: over budget.
        history.push("b2", pinned_edit(&b));
        let labels: Vec<&str> = history.undo.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["b", "b2"], "the entries pinning a are evicted; b is shared with the newest entry");
    }

    #[test]
    fn pixel_edits_undo_and_redo_through_the_document() {
        let mut doc = doc_from_raw(samples::flat_quad(), 128);
        let layer = doc.sets[0].layers[0].id;
        let s0 = doc_state(&mut doc);
        let v0 = doc.layer_version(layer).unwrap();
        assert!(!doc.can_undo() && !doc.can_redo());
        assert_eq!((doc.undo_label(), doc.redo_label()), (None, None));
        assert_eq!((doc.undo(), doc.redo()), (None, None));
        assert!(!doc.is_modified());

        paint_rect(&mut doc, layer, IRect::new(10, 10, 80, 70), [255, 0, 0, 255]);
        let v1 = doc.layer_version(layer).unwrap();
        assert_ne!(v1, v0, "a pixel edit gives the layer a fresh version");
        let s1 = doc_state(&mut doc);
        assert_eq!(doc.take_gpu_dirty(TextureSetId(0)), vec![IRect::new(10, 10, 80, 70)]);
        paint_rect(&mut doc, layer, IRect::new(60, 60, 128, 128), [0, 0, 255, 128]);
        let s2 = doc_state(&mut doc);
        let v2 = doc.layer_version(layer).unwrap();
        assert_ne!(s1, s2);
        assert!(doc.is_modified());
        assert_eq!(doc.history.undo_len(), 2);
        assert_eq!(doc.undo_label(), Some("Paint"));
        doc.take_gpu_dirty(TextureSetId(0));

        let r = doc.revision();
        assert_eq!(doc.undo().as_deref(), Some("Paint"));
        assert_eq!(doc.revision(), r + 1, "undo bumps the revision");
        let v = doc.layer_version(layer).unwrap();
        assert!(v != v2 && v != v1 && v != v0, "undo of a pixel edit gives a fresh version");
        assert_eq!(doc_state(&mut doc), s1);
        // The undo repaints exactly the tiles of the patch (the second edit touched all four).
        assert_eq!(doc.take_gpu_dirty(TextureSetId(0)), vec![IRect::new(0, 0, 128, 128)]);
        assert_eq!(doc.undo().as_deref(), Some("Paint"));
        assert_eq!(doc_state(&mut doc), s0);
        assert_eq!(doc.undo(), None);
        assert!(doc.can_redo() && !doc.can_undo());
        assert_eq!(doc.redo_label(), Some("Paint"));

        assert_eq!(doc.redo().as_deref(), Some("Paint"));
        assert_ne!(doc.layer_version(layer).unwrap(), v, "redo of a pixel edit gives a fresh version");
        assert_eq!(doc_state(&mut doc), s1);
        assert_eq!(doc.redo().as_deref(), Some("Paint"));
        assert_eq!(doc_state(&mut doc), s2);
        assert_eq!(doc.redo(), None);

        // A new edit clears the redo stack.
        doc.undo();
        assert!(doc.can_redo());
        paint_rect(&mut doc, layer, IRect::new(0, 0, 4, 4), [1, 2, 3, 255]);
        assert!(!doc.can_redo());
        assert_eq!(doc.history.redo_len(), 0);
        // Only one Arc holder: the history stores tiles, never the layer.
        assert_eq!(Arc::strong_count(raster_arc(&doc, layer)), 1);
    }

    #[test]
    fn multi_edits_apply_in_reverse_for_undo() {
        // Two patches on the same tile inside one Multi: undo must apply the second one first.
        let mut doc = doc_from_raw(samples::flat_quad(), 64);
        let layer = doc.sets[0].layers[0].id;
        let s0 = doc_state(&mut doc);
        let mut edits = Vec::new();
        for rgba in [[10, 0, 0, 255], [0, 20, 0, 255]] {
            paint_rect(&mut doc, layer, IRect::new(8, 8, 24, 24), rgba);
            edits.push(doc.history.undo.pop().unwrap().edit);
        }
        doc.push_edit("Both", Edit::Multi(edits));
        assert_eq!(doc.history.undo_len(), 1);
        let s1 = doc_state(&mut doc);
        assert_eq!(doc.undo().as_deref(), Some("Both"));
        assert_eq!(doc_state(&mut doc), s0);
        assert_eq!(doc.redo().as_deref(), Some("Both"));
        assert_eq!(doc_state(&mut doc), s1);
    }

    #[test]
    fn is_modified_and_mark_saved() {
        let mut doc = doc_from_raw(samples::flat_quad(), 64);
        let layer = doc.sets[0].layers[0].id;
        assert!(!doc.is_modified());
        assert_eq!(doc.history_state(), doc.revision(), "a fresh document: state id == revision");
        paint_rect(&mut doc, layer, IRect::new(0, 0, 8, 8), [1, 1, 1, 255]);
        assert!(doc.is_modified());
        assert_eq!(doc.history_state(), doc.revision(), "right after a push: state id == revision");
        doc.mark_saved(doc.history_state());
        assert!(!doc.is_modified());
        // Undo below the saved state: modified; redo back to it: unmodified again (R3-6).
        doc.undo();
        assert!(doc.is_modified());
        doc.redo();
        assert!(!doc.is_modified(), "redo back to the saved state clears the modified mark");
        // Save with the stroke undone; one more stroke, then undo: back at the saved state.
        doc.undo();
        doc.mark_saved(doc.history_state());
        assert!(!doc.is_modified());
        doc.redo();
        assert!(doc.is_modified(), "redo above the saved state makes it modified again");
        paint_rect(&mut doc, layer, IRect::new(8, 8, 16, 16), [3, 3, 3, 255]);
        doc.undo();
        assert!(doc.is_modified(), "still one entry above the saved state");
        doc.undo();
        assert!(!doc.is_modified(), "undo back to the saved state clears the modified mark");
        doc.redo();
        doc.redo();
        doc.undo();
        doc.undo();
        assert!(!doc.is_modified(), "any undo/redo walk that ends at the saved state is unmodified");
        // A new edit after undoing below the saved state: the saved state is gone from the redo stack for good.
        doc.redo();
        doc.mark_saved(doc.history_state());
        doc.undo();
        paint_rect(&mut doc, layer, IRect::new(0, 0, 8, 8), [1, 1, 1, 255]);
        assert!(doc.is_modified(), "same pixels as the saved state, but through a new edit");
        doc.undo();
        assert!(doc.is_modified());
        doc.redo();
        assert!(doc.is_modified());

        // A snapshot carries the state to mark as saved later; an edit made meanwhile keeps the document modified,
        // undoing that edit again before the save finishes leaves it unmodified.
        let snapshot = doc.snapshot();
        assert_eq!(snapshot.history_state, doc.history_state());
        paint_rect(&mut doc, layer, IRect::new(0, 0, 8, 8), [2, 2, 2, 255]);
        doc.mark_saved(snapshot.history_state);
        assert!(doc.is_modified());
        doc.undo();
        assert!(!doc.is_modified());

        // A pending Commit::Preview edit: not counted by is_modified (has_pending_edit is), and undoing it (flush +
        // undo) returns to the saved state.
        doc.begin_pending("Drag");
        doc.sets[0].layers[0].opacity = 0.5;
        assert!(!doc.is_modified() && doc.has_pending_edit());
        doc.undo();
        assert!(!doc.is_modified() && !doc.has_pending_edit());
    }

    #[test]
    fn an_evicted_saved_state_stays_modified() {
        let mut doc = doc_from_raw(samples::flat_quad(), 64);
        doc.history.max_entries = 2;
        let layer = doc.sets[0].layers[0].id;
        doc.mark_saved(doc.history_state());
        for i in 0..3u8 {
            paint_rect(&mut doc, layer, IRect::new(0, 0, 8, 8), [i, i, i, 255]);
        }
        assert_eq!(doc.history.undo_len(), 2, "the first entry (whose before-state is the saved one) is evicted");
        while doc.undo().is_some() {
            assert!(doc.is_modified(), "the saved state cannot be reached any more");
        }
        // The oldest reachable state is the one after the evicted entry; saving there and walking back works.
        doc.mark_saved(doc.history_state());
        doc.redo();
        doc.redo();
        assert!(doc.is_modified());
        doc.undo();
        doc.undo();
        assert!(!doc.is_modified());
        // Clearing the history keeps the current state's id.
        doc.redo();
        doc.mark_saved(doc.history_state());
        doc.history.clear();
        assert!(!doc.is_modified());
    }

    #[test]
    fn history_state_ids_are_unique() {
        // Every pushed state gets a never-used id, so no unrelated state can compare equal to the saved one.
        let mut doc = doc_from_raw(samples::flat_quad(), 64);
        let layer = doc.sets[0].layers[0].id;
        let mut ids = vec![doc.history_state()];
        for i in 0..3u8 {
            paint_rect(&mut doc, layer, IRect::new(0, 0, 4, 4), [i, 0, 0, 255]);
            assert!(!ids.contains(&doc.history_state()));
            ids.push(doc.history_state());
        }
        for k in (0..3).rev() {
            doc.undo();
            assert_eq!(doc.history_state(), ids[k], "undo returns to the earlier state's id");
        }
        for id in &ids[1..] {
            doc.redo();
            assert_eq!(doc.history_state(), *id, "redo returns to the later state's id");
        }
        doc.undo();
        doc.undo();
        paint_rect(&mut doc, layer, IRect::new(0, 0, 4, 4), [1, 0, 0, 255]);
        assert!(!ids.contains(&doc.history_state()), "a new edit after undo gets a fresh id");
    }

    #[test]
    fn pending_edit_mechanics() {
        let full = IRect::from_size(SIZE, SIZE);
        let mut doc = doc_with_stamp();
        let s0 = doc_state(&mut doc);
        assert!(!doc.has_pending_edit());
        assert!(!doc.cancel_pending_edit());
        doc.flush_pending_edit();
        assert_eq!(doc.history.undo_len(), 0);

        // The first begin captures; the same label continues; the document is not "modified" yet.
        doc.begin_pending("Drag");
        assert!(doc.has_pending_edit());
        doc.sets[0].layers[0].opacity = 0.5;
        doc.sets[0].mark_dirty(full);
        doc.begin_pending("Drag");
        doc.sets[0].layers[0].opacity = 0.25;
        doc.sets[0].mark_dirty(full);
        assert_eq!(doc.history.undo_len(), 0);
        assert!(!doc.is_modified());
        assert!(doc.can_undo(), "undo would flush and undo the pending edit");
        assert_eq!(doc.undo_label(), Some("Drag"));
        assert!(!doc.can_redo());
        let s1 = doc_state(&mut doc);
        assert_ne!(s0, s1);

        // Cancel restores the state from before the first begin.
        assert!(doc.cancel_pending_edit());
        assert!(!doc.has_pending_edit());
        assert_eq!(doc_state(&mut doc), s0);
        assert_eq!(doc.history.undo_len(), 0);
        assert!(!doc.is_modified());

        // Flush pushes exactly one entry holding the "before" state.
        doc.begin_pending("Drag");
        doc.sets[0].layers[0].opacity = 0.25;
        doc.sets[0].mark_dirty(full);
        doc.flush_pending_edit();
        assert!(!doc.has_pending_edit());
        assert_eq!(doc.history.undo_len(), 1);
        assert!(doc.is_modified());
        assert_eq!(doc_state(&mut doc), s1);
        doc.flush_pending_edit();
        assert_eq!(doc.history.undo_len(), 1);

        // A different label flushes the open edit first.
        doc.begin_pending("Drag");
        doc.sets[0].layers[0].opacity = 0.75;
        doc.sets[0].mark_dirty(full);
        doc.begin_pending("Other");
        assert_eq!(doc.history.undo_len(), 2);
        assert_eq!(doc.undo_label(), Some("Other"));
        // undo() flushes the pending edit and undoes it (nothing changed during "Other").
        let s2 = doc_state(&mut doc);
        assert_eq!(doc.undo().as_deref(), Some("Other"));
        assert_eq!(doc_state(&mut doc), s2);
        assert_eq!(doc.undo().as_deref(), Some("Drag"));
        assert_eq!(doc_state(&mut doc), s1);
        assert_eq!(doc.undo().as_deref(), Some("Drag"));
        assert_eq!(doc_state(&mut doc), s0);
        // redo() with a pending edit: the flush is a new entry and clears the redo stack.
        assert_eq!(doc.history.redo_len(), 3);
        doc.begin_pending("Drag");
        assert!(!doc.can_redo());
        assert_eq!(doc.redo(), None);
        assert_eq!((doc.history.undo_len(), doc.history.redo_len()), (1, 0));
    }

    #[test]
    fn snapshot_pins_only_what_is_asked_and_restore_returns_the_inverse() {
        let mut doc = doc_with_stamp();
        let s0 = doc_state(&mut doc);
        let snapshot = doc.snapshot_structure(&[LayerId(3)]);
        assert_eq!(
            record_kinds(&snapshot),
            [
                vec![(LayerId(1), "keep"), (LayerId(101), "stamp"), (LayerId(3), "raster")],
                vec![(LayerId(2), "keep"), (LayerId(102), "stamp")]
            ]
        );
        assert_eq!(snapshot.stamps, doc.stamps);
        assert_eq!(snapshot.sets[0].active, Some(LayerId(3)));
        assert_eq!(Arc::strong_count(raster_arc(&doc, LayerId(1))), 1);
        assert_eq!(Arc::strong_count(raster_arc(&doc, LayerId(3))), 2);
        drop(snapshot);

        // Restoring the current structure changes nothing, marks nothing dirty and pins nothing.
        let versions = |doc: &Document| doc.sets.iter().flat_map(|s| &s.layers).map(|l| l.version()).collect::<Vec<u64>>();
        let v0 = versions(&doc);
        let same = doc.snapshot_structure(&[]);
        let inverse = doc.restore_structure(same);
        assert!(!doc.has_pending_composite());
        assert_eq!(doc_state(&mut doc), s0);
        assert_eq!(versions(&doc), v0);
        assert!(record_kinds(&inverse).iter().flatten().all(|(_, kind)| *kind != "raster"));
        drop(inverse);
        for id in [1, 2, 3] {
            assert_eq!(Arc::strong_count(raster_arc(&doc, LayerId(id))), 1);
        }

        // A property change of the stamp layer dirties exactly its cache rect, in its set only.
        let before = doc.snapshot_structure(&[]);
        doc.sets[0].layers[1].visible = false;
        doc.sets[0].mark_dirty(IRect::new(10, 12, 50, 40));
        let s_hidden = doc_state(&mut doc);
        let inverse = doc.restore_structure(before);
        assert_eq!(doc.sets[0].pending.rects(), [IRect::new(10, 12, 50, 40)]);
        assert!(doc.sets[1].pending.is_empty());
        assert_eq!(doc_state(&mut doc), s0);
        assert_eq!(versions(&doc), v0, "property changes never touch layer versions");
        doc.restore_structure(inverse);
        assert_eq!(doc.sets[0].pending.rects(), [IRect::new(10, 12, 50, 40)]);
        assert_eq!(doc_state(&mut doc), s_hidden);

        // Removing layers and the stamp by hand, with the removed paint layer pinned as a command would do:
        // restore brings everything back (the pinned buffer with its stored version), the inverse removes it again.
        let before = doc.snapshot_structure(&[LayerId(3)]);
        doc.sets[0].layers.remove(2);
        doc.sets[0].layers.remove(1);
        doc.sets[0].active = Some(LayerId(1));
        doc.sets[1].layers.remove(1);
        doc.stamps.clear();
        doc.sets[0].mark_dirty(IRect::from_size(SIZE, SIZE));
        doc.sets[1].mark_dirty(IRect::from_size(SIZE, SIZE));
        let s_removed = doc_state(&mut doc);
        let inverse = doc.restore_structure(before);
        assert_eq!(record_kinds(&inverse), [vec![(LayerId(1), "keep")], vec![(LayerId(2), "keep")]]);
        // Added back: the stamp layers' rects (paint layer 3 is transparent: its content bound is empty).
        assert_eq!(doc.sets[0].pending.rects(), [IRect::new(10, 12, 50, 40)]);
        assert_eq!(doc.sets[1].pending.rects(), [IRect::new(60, 5, 90, 30)]);
        assert_eq!(doc_state(&mut doc), s_hidden);
        assert_eq!(versions(&doc), v0);
        let back = doc.restore_structure(inverse);
        assert_eq!(doc_state(&mut doc), s_removed);
        // The layer that disappeared again is pinned in the returned snapshot, the kept ones are not.
        assert_eq!(
            record_kinds(&back),
            [
                vec![(LayerId(1), "keep"), (LayerId(101), "stamp"), (LayerId(3), "raster")],
                vec![(LayerId(2), "keep"), (LayerId(102), "stamp")]
            ]
        );
    }

    #[test]
    fn reordering_dirties_only_the_layers_that_changed_places() {
        let mut doc = doc_with_stamp();
        doc_state(&mut doc);
        let before = doc.snapshot_structure(&[]);
        let mut other = before.clone();
        // Swap the stamp layer of set 0 with the transparent paint layer above it: the content of both is dirty
        // (the paint layer has none).
        other.sets[0].layers.swap(1, 2);
        doc.restore_structure(other);
        assert_eq!(doc.sets[0].pending.rects(), [IRect::new(10, 12, 50, 40)]);
        assert!(doc.sets[1].pending.is_empty(), "set 1 did not change");
        doc_state(&mut doc);
        // Only `active` changes: nothing is dirty.
        let mut other = doc.snapshot_structure(&[]);
        other.sets[0].active = Some(LayerId(1));
        other.sets[0].layers[0].name = "renamed".to_owned();
        other.sets[0].layers[0].locked = true;
        doc.restore_structure(other);
        assert!(!doc.has_pending_composite());
        assert_eq!(doc.sets[0].active, Some(LayerId(1)));
        assert_eq!(doc.sets[0].layers[0].name, "renamed");
        drop(before);
    }

    #[test]
    fn restore_drops_stale_text_rasters() {
        use crate::ids::BlobId;
        use crate::pixel_ops::PremulMips;
        use crate::text::{FontRef, TextAlign, TextSpec};

        let mut doc = doc_with_stamp();
        let spec = TextSpec {
            content: "hi".to_owned(),
            font: FontRef { blob: BlobId(0), face_index: 0, family: "f".to_owned(), style: "Regular".to_owned() },
            size_px: 64.0,
            color: [0, 0, 0, 255],
            align: TextAlign::Left,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: None,
            fx: crate::text::TextFx::default(),
        };
        let mips = Arc::new(PremulMips::build(&PixelBuf::new(4, 4)));
        // Stamp 100 is an image stamp: a cached text raster for it is stale. Stamp 555 does not exist.
        doc.text_mips.insert(STAMP, (spec.clone(), Arc::clone(&mips)));
        doc.text_mips.insert(StampId(555), (spec.clone(), Arc::clone(&mips)));
        let same = doc.snapshot_structure(&[]);
        doc.restore_structure(same);
        assert!(doc.text_mips.is_empty());

        // A text stamp whose spec still matches keeps its raster; a changed spec drops it.
        doc.stamps[0].source = StampSource::Text(spec.clone());
        for set in &mut doc.sets {
            for layer in &mut set.layers {
                if let LayerKind::Stamp { stamp, cache } = &layer.kind {
                    layer.kind = LayerKind::Text { stamp: *stamp, cache: Arc::clone(cache) };
                }
            }
        }
        doc.text_mips.insert(STAMP, (spec.clone(), Arc::clone(&mips)));
        let same = doc.snapshot_structure(&[]);
        doc.restore_structure(same);
        assert_eq!(doc.text_mips.len(), 1);
        let mut changed = spec;
        changed.content = "ho".to_owned();
        let mut other = doc.snapshot_structure(&[]);
        other.stamps[0].source = StampSource::Text(changed);
        doc.restore_structure(other);
        assert!(doc.text_mips.is_empty());
    }
}
