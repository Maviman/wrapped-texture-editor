//! The round trip between a sticker and its PSD (STICKER_PLAN 5.4 with the ids rules of 13.2; ARCHITECTURE 6.13).
//!
//! **WTE -> PSD** ([`export`]): one PSD pixel layer per sticker layer, bottom to top, at the link scale `s`
//! ([`link_scale`]): the layer's content with its adjustments and transform, **without** its styles and its mask,
//! clipped to the canvas and normalised (zero RGB under alpha 0, trimmed); a picture layer's mask is written as the
//! PSD layer mask, placed the same way; `clip`, name, visibility, opacity and blend are carried over. A layer already
//! in the map keeps its `lyid` for ever, a new one gets `next_lyid`.
//!
//! **PSD -> WTE** ([`import`]): with nothing forcing (`PsdDoc::forcing`), layer by layer by the table of 5.4 after the
//! three ids rules of 13.2 (no known id, renumbered, one layer re-numbered) and the "`lyid` reuse" rule; otherwise
//! (or for a flat file) as one merged picture. The comparison rule (review A11): a PSD layer is clipped to the canvas
//! and normalised, then its size and the CRC-32 of its bytes are compared with the map entry, its origin separately
//! (equal bytes elsewhere = moved), its mask as values over the layer's rectangle (with the disabled flag).
//!
//! Pure functions of their inputs: new pictures and masks carry version 0 (the document gives them fresh versions
//! when it takes the sticker, `Document::apply_link_update`).

use std::collections::HashSet;
use std::sync::Arc;

use glam::Vec2;

use crate::composite::opacity_u8;
use crate::ids::{BlobId, StickerLayerId};
use crate::math::IRect;
use crate::pixel::{AlphaBuf, CroppedBuf, PixelBuf};
use crate::psd::write::normalise;
use crate::psd::{PsdBlend, PsdDoc, PsdError, PsdFeatures, PsdLayer, PsdMask, StickerPsd, StickerPsdLayer};
use crate::sticker::render::{StickerRenderInput, layer_content, mask_raster};
use crate::sticker::{
    LayerMask, LayerTransform, MAX_ARTBOARD, MAX_OWNED_BYTES, MAX_STICKER_LAYERS, PicturePixels, StickerContent, StickerDoc,
    StickerLayer,
};
use crate::text::RasterQuality;

use super::{LinkLayer, LinkMode, PLACEHOLDER_LAYER, StickerLink, smart_forces, texts};

/// The link scale of 5.4 (review B4): the larger of the sticker's Final render scale (`final_scale`, 2.6; the
/// largest over its stamps) and `ceil(1 / smallest picture-layer scale)`, rounded up to 1, 2 or 4 (at most 4), then
/// halved until the canvas `artboard * s` is at most 4096 a side.
pub fn link_scale(doc: &StickerDoc, final_scale: f32) -> u32 {
    let mut need = if final_scale.is_finite() && final_scale > 1.0 { final_scale } else { 1.0 };
    for l in &doc.layers {
        if matches!(l.content, StickerContent::Picture(_)) {
            let m = l.transform.scale.x.abs().min(l.transform.scale.y.abs());
            if m.is_finite() && m > 0.0 && m < 1.0 {
                need = need.max((1.0 / m).ceil());
            }
        }
    }
    let mut s = [1u32, 2, 4].into_iter().find(|&c| c as f32 >= need).unwrap_or(4);
    let [w, h] = doc.size();
    while s > 1 && (w.saturating_mul(s) > MAX_ARTBOARD || h.saturating_mul(s) > MAX_ARTBOARD) {
        s /= 2;
    }
    s
}

/// CRC-32 of a mask as the comparison rule sees it: the disabled flag, then the mask's values over `rect` row by
/// row (the mask's own rectangle does not matter, only what it says over the layer). `None` without a mask.
pub fn mask_crc(mask: Option<&PsdMask>, rect: IRect) -> Option<u32> {
    let mask = mask?;
    let mut h = crc32fast::Hasher::new();
    h.update(&[mask.disabled as u8]);
    let mut row = Vec::with_capacity(rect.width() as usize);
    for y in rect.y0..rect.y1 {
        row.clear();
        row.extend((rect.x0..rect.x1).map(|x| mask.value(x, y)));
        h.update(&row);
    }
    Some(h.finalize())
}

/// What [`export`] made: the PSD to write and the link data that goes with it.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkExport {
    pub psd: StickerPsd,
    pub layer_map: Vec<LinkLayer>,
    pub next_lyid: u32,
    /// The link scale used (1, 2 or 4).
    pub scale: u32,
}

/// The part of `buf` inside `r`.
fn crop(buf: &CroppedBuf, r: IRect) -> CroppedBuf {
    let rect = buf.rect.intersect(&r);
    if rect == buf.rect {
        return buf.clone();
    }
    let mut out = CroppedBuf::new(rect);
    if rect.is_empty() {
        return out;
    }
    let (sw, w) = (buf.rect.width() as usize * 4, rect.width() as usize * 4);
    for (y, dst) in (rect.y0..rect.y1).zip(out.data.chunks_exact_mut(w)) {
        let s = (y - buf.rect.y0) as usize * sw + (rect.x0 - buf.rect.x0) as usize * 4;
        dst.copy_from_slice(&buf.data[s..s + w]);
    }
    out
}

/// A PSD mask from a `mask_raster` buffer: the red channel where the picture lands, 255 (shown) elsewhere.
fn psd_mask_of(raster: &CroppedBuf) -> PsdMask {
    let r = raster.rect;
    let data = raster.data.as_chunks::<4>().0.iter().map(|p| if p[3] > 0 { p[0] } else { 255 }).collect();
    PsdMask {
        rect: r,
        default: 255,
        pixels: AlphaBuf { width: r.width(), height: r.height(), data },
        disabled: false,
        rendered: false,
    }
}

/// The empty layer written for a sticker without layers (a PSD needs one, check-P5b finding 1).
const PLACEHOLDER_NAME: &str = "Layer 1";

/// WTE -> PSD (module docs). `input` holds the sticker with its decoded blob pictures and the fonts of **every** text
/// layer (hidden ones too: they are written); `scale` is the link scale; `previous` the link whose ids are kept.
/// Errors only for what a PSD cannot hold (`PsdError::Invalid`, from the writer's checks at write time).
pub fn export(input: &StickerRenderInput, scale: u32, previous: Option<&StickerLink>) -> Result<LinkExport, PsdError> {
    let doc = &*input.doc;
    let s = scale.clamp(1, 4);
    let [w, h] = doc.size();
    let canvas = [w.saturating_mul(s), h.saturating_mul(s)];
    if canvas[0] == 0 || canvas[1] == 0 {
        return Err(PsdError::Invalid("the sticker has an empty artboard"));
    }
    let clip = IRect::from_size(canvas[0], canvas[1]);
    let mut next = previous.map_or(1, |l| l.next_lyid.max(1));
    let mut used: HashSet<u32> = HashSet::new();
    let mut layers = Vec::with_capacity(doc.layers.len().max(1));
    let mut map = Vec::with_capacity(doc.layers.len().max(1));
    let mut push = |name: &str, layer_id: StickerLayerId, kept: Option<u32>, pixels: CroppedBuf, mask: Option<PsdMask>, layer: Option<&StickerLayer>| {
        let lyid = match kept.filter(|id| *id >= 1 && !used.contains(id)) {
            Some(id) => id,
            None => {
                while used.contains(&next) {
                    next += 1;
                }
                next
            }
        };
        used.insert(lyid);
        next = next.max(lyid + 1);
        let pixels = normalise(&pixels);
        let origin = if pixels.is_empty() { [0, 0] } else { [pixels.rect.x0, pixels.rect.y0] };
        map.push(LinkLayer {
            lyid,
            layer: layer_id,
            name: name.to_owned(),
            origin,
            size: [pixels.rect.width(), pixels.rect.height()],
            pixels_crc: crc32fast::hash(&pixels.data),
            mask_crc: mask_crc(mask.as_ref(), pixels.rect),
            wte: None,
        });
        layers.push(StickerPsdLayer {
            name: name.to_owned(),
            lyid,
            pixels,
            mask,
            clipped: layer.is_some_and(|l| l.clip),
            visible: layer.is_none_or(|l| l.visible),
            opacity: layer.map_or(255, |l| opacity_u8(l.opacity)),
            blend: layer.map_or(crate::layer::BlendMode::Normal, |l| l.blend),
        });
    };
    for layer in &doc.layers {
        let kept = previous.and_then(|l| l.layer_map.iter().find(|e| e.layer == layer.id)).map(|e| e.lyid);
        let content = layer_content(input, layer, s as f32, RasterQuality::Full, clip).unwrap_or_else(CroppedBuf::empty);
        let mask = mask_raster(input, layer, s as f32, clip).map(|m| psd_mask_of(&m));
        push(&layer.name, layer.id, kept, content, mask, Some(layer));
    }
    if doc.layers.is_empty() {
        let kept = previous.and_then(|l| l.layer_map.iter().find(|e| e.layer == PLACEHOLDER_LAYER)).map(|e| e.lyid);
        push(PLACEHOLDER_NAME, PLACEHOLDER_LAYER, kept, CroppedBuf::empty(), None, None);
    }
    Ok(LinkExport { psd: StickerPsd { size: canvas, layers, id_seed: next - 1 }, layer_map: map, next_lyid: next, scale: s })
}

/// Names and facts [`import`] needs besides the file and the sticker.
#[derive(Copy, Clone, Debug)]
pub struct ImportContext<'a> {
    /// The link editor's key ("photocraft", "photoshop", "other"): decides whether SMART forces (13.2).
    pub editor_key: &'a str,
    /// The link editor's name, for the merged layer's name and the toast.
    pub editor_name: &'a str,
    /// The file's name, for the profile note and the log line.
    pub file_name: &'a str,
    /// Decoded sizes of the sticker's blob pictures (a mask change on an unedited picture keeps the picture).
    pub blob_sizes: &'a [(BlobId, [u32; 2])],
    /// `Document::link_layer_hash` of the sticker's current layers (check-P5c): a matched layer whose hash differs
    /// from its map entry's `wte` changed in WTE since the sync and takes the file's pixels. A layer missing here (or
    /// an entry without `wte`) is trusted.
    pub layer_hashes: &'a [(StickerLayerId, u64)],
}

/// Why [`import`] applied nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportProblem {
    /// Something forces the merged path and the file has no real merged picture.
    NeedsMerged,
    /// The canvas divided by the link scale is larger than a sticker may be; offer [`import_fitted`].
    CanvasTooLarge { width: u32, height: u32 },
    /// The canvas is over the reader's pixel limits.
    TooLarge { width: u32, height: u32 },
}

/// What happened to one layer (tests and the log); PSD layers bottom to top, then the sticker layers that went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerOutcome {
    /// Kept as it was (text stays text); name, visibility, opacity, blend, clip and order taken.
    Untouched(StickerLayerId),
    /// Kept and moved by the PSD origin's change.
    Moved(StickerLayerId),
    /// The mask changed: `kept` = the layer stayed (a whole-pixel picture at scale 1) and only its mask was replaced;
    /// else it became a picture layer with the PSD pixels and mask.
    MaskChanged { layer: StickerLayerId, kept: bool },
    /// Became a picture layer with the PSD pixels (styles kept).
    PixelsChanged(StickerLayerId),
    /// A layer made in the editor (or one the ids rules could not match).
    New(StickerLayerId),
    /// The placeholder layer of an empty sticker, still empty: nothing.
    Placeholder,
    /// A sticker layer that is not in the PSD any more.
    Deleted(StickerLayerId),
}

/// What [`import`] made.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkImport {
    /// The new sticker value (same name, shape and `keep`; artboard from the canvas).
    pub sticker: StickerDoc,
    pub mode: LinkMode,
    pub scale: f32,
    pub layer_map: Vec<LinkLayer>,
    pub next_lyid: u32,
    /// Some layer is the editor's own text, smart object or fill layer.
    pub editor_native: bool,
    /// The ids rule that made the ids untrusted (13.2), when one did.
    pub ids_lost: Option<&'static str>,
    pub outcomes: Vec<LayerOutcome>,
    /// The merged toast (5.4), when the file came back as one picture for a reason.
    pub toast: Option<String>,
    /// The colour-profile note of 13.2, when the file has a non-sRGB profile.
    pub profile_note: Option<String>,
    /// Lines for the log ("layer ids of <file> were not kept (<rule>)").
    pub log: Vec<String>,
}

/// One PSD layer as the comparison rule sees it.
struct Read<'a> {
    layer: &'a PsdLayer,
    /// The layer's id; `None` for a layer without one and for a later duplicate (13.2 "Duplicate ids").
    id: Option<u32>,
    pixels: CroppedBuf,
    crc: u32,
    size: [u32; 2],
    origin: [i32; 2],
    mask_crc: Option<u32>,
}

impl Read<'_> {
    fn of(layer: &PsdLayer, canvas: IRect) -> Read<'_> {
        let pixels = layer.pixels.as_ref().map_or_else(CroppedBuf::empty, |p| normalise(&crop(p, canvas)));
        let origin = if pixels.is_empty() { [0, 0] } else { [pixels.rect.x0, pixels.rect.y0] };
        Read {
            layer,
            id: layer.lyid,
            crc: crc32fast::hash(&pixels.data),
            size: [pixels.rect.width(), pixels.rect.height()],
            origin,
            mask_crc: mask_crc(layer.mask.as_ref(), pixels.rect),
            pixels,
        }
    }

    /// Same normalised size and bytes as the entry.
    fn same_pixels(&self, e: &LinkLayer) -> bool {
        self.size == e.size && self.crc == e.pixels_crc
    }

    /// The map entry this layer becomes.
    fn entry(&self, lyid: u32, layer: StickerLayerId) -> LinkLayer {
        LinkLayer {
            lyid,
            layer,
            name: self.layer.name.clone(),
            origin: self.origin,
            size: self.size,
            pixels_crc: self.crc,
            mask_crc: self.mask_crc,
            wte: None,
        }
    }
}

/// How a matched layer differs from its entry (the rows of 5.4).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Change {
    Untouched,
    Moved,
    Mask,
    Pixels,
}

fn change(r: &Read<'_>, e: &LinkLayer) -> Change {
    if !r.same_pixels(e) {
        Change::Pixels
    } else if r.mask_crc != e.mask_crc {
        Change::Mask
    } else if r.origin != e.origin {
        Change::Moved
    } else {
        Change::Untouched
    }
}

/// Per PSD layer: the index of the map entry it is and whether it was matched by name only (pass 2 of 13.2: its
/// pixels changed); `None` = a new layer.
type Assignment = Vec<Option<(usize, bool)>>;

/// The matching of 13.2: per PSD layer, the map entry it is, and the ids rule that fired.
fn match_layers(reads: &[Read<'_>], map: &[LinkLayer]) -> (Assignment, Option<&'static str>) {
    let entry_of = |id: u32| map.iter().position(|e| e.lyid == id);
    let mut assign: Assignment = vec![None; reads.len()];
    if map.is_empty() || reads.is_empty() {
        return (assign, None);
    }
    let mut claimed = vec![false; map.len()];
    // Rule 1: the file has layers and none carries a known id.
    let no_known = !reads.iter().any(|r| r.id.and_then(entry_of).is_some());
    // Rule 2: a layer with a known id whose pixels and name both differ from that entry while its pixels (or, P5c
    // addition, its name) equal another entry (Photoshop numbering 1..n by position would look like this; the name
    // catches it when the layer that took a moved id was also repainted).
    let renumbered = reads.iter().any(|r| {
        let Some(i) = r.id.and_then(entry_of) else { return false };
        let e = &map[i];
        !r.same_pixels(e)
            && r.layer.name != e.name
            && map.iter().enumerate().any(|(j, f)| j != i && (r.same_pixels(f) || (!f.name.is_empty() && r.layer.name == f.name)))
    });
    let lost = if no_known {
        Some("no layer carries a known id")
    } else if renumbered {
        Some("the ids were renumbered")
    } else {
        None
    };
    if lost.is_none() {
        for (k, r) in reads.iter().enumerate() {
            let Some(i) = r.id.and_then(entry_of) else { continue };
            let e = &map[i];
            // The "lyid reuse" rule (5.4, fixture f12): other pixels **and** another name = the mapped layer was
            // deleted and a new layer took its id.
            if claimed[i] || (!r.same_pixels(e) && r.layer.name != e.name) {
                continue;
            }
            claimed[i] = true;
            assign[k] = Some((i, false));
        }
        // Rule 3: an unknown id whose pixels and name equal an entry whose own id is not in the file is that layer
        // under a new id.
        let file_ids: HashSet<u32> = reads.iter().filter_map(|r| r.layer.lyid).collect();
        for (k, r) in reads.iter().enumerate() {
            if assign[k].is_some() || r.id.and_then(entry_of).is_some() {
                continue;
            }
            if let Some(i) = (0..map.len())
                .find(|&i| !claimed[i] && !file_ids.contains(&map[i].lyid) && r.same_pixels(&map[i]) && r.layer.name == map[i].name)
            {
                claimed[i] = true;
                assign[k] = Some((i, false));
            }
        }
        return (assign, None);
    }
    // Ids lost. Pass 1: by pixels (CRC and size), ties broken by name, then by order.
    for (k, r) in reads.iter().enumerate() {
        let candidates: Vec<usize> = (0..map.len()).filter(|&i| !claimed[i] && r.same_pixels(&map[i])).collect();
        let pick = candidates.iter().copied().find(|&i| map[i].name == r.layer.name).or(candidates.first().copied());
        if let Some(i) = pick {
            claimed[i] = true;
            assign[k] = Some((i, false));
        }
    }
    // Pass 2: a still-unmatched layer whose name equals the name of exactly one still-unmatched entry is that layer
    // with changed pixels.
    for (k, r) in reads.iter().enumerate() {
        if assign[k].is_some() {
            continue;
        }
        let same: Vec<usize> = (0..map.len()).filter(|&i| !claimed[i] && map[i].name == r.layer.name).collect();
        if let [i] = same[..] {
            claimed[i] = true;
            assign[k] = Some((i, true));
        }
    }
    (assign, lost)
}

/// The transform that shows a picture of `size` whose top-left sits at canvas pixel `canvas_origin` at its place:
/// scale `1 / s`, top-left at `artboard origin + canvas_origin / s` (5.4: nothing is resampled until render).
fn placed(size: [u32; 2], canvas_origin: [i32; 2], s: f32, artboard: IRect) -> LayerTransform {
    let o = Vec2::new(artboard.x0 as f32 + canvas_origin[0] as f32 / s, artboard.y0 as f32 + canvas_origin[1] as f32 / s);
    if s == 1.0 {
        return LayerTransform::moved(o.x, o.y);
    }
    let c = Vec2::new(size[0] as f32, size[1] as f32) * 0.5;
    LayerTransform { translate: o - c * (1.0 - 1.0 / s), scale: Vec2::splat(1.0 / s), rotation: 0.0 }
}

/// An owned picture of `pixels` (a 1 x 1 transparent picture for an empty layer) and where it sits.
fn picture_of(pixels: &CroppedBuf) -> (PixelBuf, [i32; 2]) {
    if pixels.is_empty() {
        return (PixelBuf::new(1, 1), [0, 0]);
    }
    let r = pixels.rect;
    (PixelBuf { width: r.width(), height: r.height(), data: pixels.data.clone() }, [r.x0, r.y0])
}

/// The PSD mask over `rect` as a picture mask (a disabled mask is "no mask", 5.4).
fn mask_over(mask: Option<&PsdMask>, rect: IRect) -> Option<LayerMask> {
    let mask = mask.filter(|m| !m.disabled)?;
    if rect.is_empty() {
        return None;
    }
    let mut data = Vec::with_capacity(rect.area() as usize);
    for y in rect.y0..rect.y1 {
        data.extend((rect.x0..rect.x1).map(|x| mask.value(x, y)));
    }
    Some(LayerMask { pixels: Arc::new(AlphaBuf { width: rect.width(), height: rect.height(), data }), version: 0 })
}

/// A picture layer's content as the PSD pixels: content, transform, mask; adjustments removed (they were part of
/// the pixels WTE sent).
fn become_picture(l: &mut StickerLayer, r: &Read<'_>, s: f32, artboard: IRect) {
    let (pic, origin) = picture_of(&r.pixels);
    l.transform = placed([pic.width, pic.height], origin, s, artboard);
    l.mask = if r.pixels.is_empty() { None } else { mask_over(r.layer.mask.as_ref(), r.pixels.rect) };
    l.content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(pic), version: 0 });
    l.adjustments.clear();
}

/// Name, visibility, opacity (kept when it rounds to the PSD's byte), blend (one of WTE's seven) and clip (not on
/// the bottom layer: the writer writes it unclipped) from the PSD.
fn take_props(l: &mut StickerLayer, p: &PsdLayer, bottom: bool) {
    l.name.clone_from(&p.name);
    l.visible = p.visible;
    if opacity_u8(l.opacity) != p.opacity {
        l.opacity = p.opacity as f32 / 255.0;
    }
    if let PsdBlend::Known(b) = p.blend {
        l.blend = b;
    }
    if !bottom {
        l.clip = p.clipped;
    }
}

/// The size of a picture layer's pixels, when known without decoding.
fn picture_size(l: &StickerLayer, blobs: &[(BlobId, [u32; 2])]) -> Option<[u32; 2]> {
    match &l.content {
        StickerContent::Picture(PicturePixels::Owned { pixels, .. }) => Some([pixels.width, pixels.height]),
        StickerContent::Picture(PicturePixels::Blob(b)) => blobs.iter().find(|x| x.0 == *b).map(|x| x.1),
        _ => None,
    }
}

/// Row 3 of 5.4: a picture whose transform is a whole-pixel move at scale 1 keeps its pixels and gets the PSD mask
/// mapped back into picture space. `false` when the layer is not such a picture.
fn replace_mask(l: &mut StickerLayer, r: &Read<'_>, s: f32, artboard: IRect, blobs: &[(BlobId, [u32; 2])]) -> bool {
    let t = l.transform;
    let whole = s == 1.0 && t.scale == Vec2::ONE && t.rotation == 0.0 && t.translate.fract() == Vec2::ZERO;
    let Some([w, h]) = picture_size(l, blobs).filter(|_| whole) else { return false };
    l.mask = match r.layer.mask.as_ref().filter(|m| !m.disabled) {
        None => None,
        Some(m) => {
            let (dx, dy) = (t.translate.x as i32 - artboard.x0, t.translate.y as i32 - artboard.y0);
            let mut data = Vec::with_capacity(w as usize * h as usize);
            for y in 0..h as i32 {
                data.extend((0..w as i32).map(|x| m.value(x + dx, y + dy)));
            }
            Some(LayerMask { pixels: Arc::new(AlphaBuf { width: w, height: h, data }), version: 0 })
        }
    };
    true
}

/// The artboard of a canvas: `canvas / s` rounded up, its top-left at the old top-left; `None` over the limit.
fn artboard_of(canvas: [u32; 2], s: f32, old: IRect) -> Option<IRect> {
    let side = |v: u32| ((v as f64 / s as f64).ceil() as u32).max(1);
    let (w, h) = (side(canvas[0]), side(canvas[1]));
    (w <= MAX_ARTBOARD && h <= MAX_ARTBOARD).then(|| IRect::from_xywh(old.x0, old.y0, w, h))
}

/// PSD -> WTE (module docs; the tables of 5.4 and 13.2). Nothing is applied when the result is an
/// [`ImportProblem`].
pub fn import(psd: &PsdDoc, sticker: &StickerDoc, link: &StickerLink, cx: &ImportContext<'_>) -> Result<LinkImport, ImportProblem> {
    if psd.too_large {
        return Err(ImportProblem::TooLarge { width: psd.size[0], height: psd.size[1] });
    }
    let s = link.scale_or_one();
    let artboard = artboard_of(psd.size, s, sticker.artboard)
        .ok_or(ImportProblem::CanvasTooLarge { width: psd.size[0], height: psd.size[1] })?;
    let forcing = psd.forcing(smart_forces(cx.editor_key));
    if psd.is_flat() || !forcing.is_empty() {
        return merged(psd, sticker, link, cx, s, artboard, forcing, &[]);
    }
    let out = layers(psd, sticker, link, cx, s, artboard);
    if out.sticker.layers.len() > MAX_STICKER_LAYERS {
        return merged(psd, sticker, link, cx, s, artboard, forcing, &["more than 32 layers"]);
    }
    if out.sticker.owned_bytes() > MAX_OWNED_BYTES {
        return merged(psd, sticker, link, cx, s, artboard, forcing, &["more picture data than a sticker can hold"]);
    }
    Ok(out)
}

/// "Fit it into the sticker" (5.4, review A18): one merged picture layer holding the PSD's pixels, the scale
/// `max(W, H) / 4096` so that the artboard is the canvas divided by it and no pixel is thrown away.
pub fn import_fitted(psd: &PsdDoc, sticker: &StickerDoc, link: &StickerLink, cx: &ImportContext<'_>) -> Result<LinkImport, ImportProblem> {
    if psd.too_large {
        return Err(ImportProblem::TooLarge { width: psd.size[0], height: psd.size[1] });
    }
    let [cw, ch] = psd.size;
    let s = (cw.max(ch) as f32 / MAX_ARTBOARD as f32).max(1.0);
    let side = |v: u32| (((v as f64 * MAX_ARTBOARD as f64) / cw.max(ch) as f64).ceil() as u32).clamp(1, MAX_ARTBOARD);
    let (w, h) = if s > 1.0 { (side(cw), side(ch)) } else { (cw.max(1), ch.max(1)) };
    let artboard = IRect::from_xywh(sticker.artboard.x0, sticker.artboard.y0, w, h);
    merged(psd, sticker, link, cx, s, artboard, PsdFeatures::empty(), &[])
}

/// The merged path: the sticker's layers replaced by one picture layer "<editor> picture".
#[expect(clippy::too_many_arguments, reason = "one step of import with its shared inputs")]
fn merged(
    psd: &PsdDoc,
    sticker: &StickerDoc,
    link: &StickerLink,
    cx: &ImportContext<'_>,
    s: f32,
    artboard: IRect,
    forcing: PsdFeatures,
    extra: &[&str],
) -> Result<LinkImport, ImportProblem> {
    let merged = psd.merged.as_ref().ok_or(ImportProblem::NeedsMerged)?;
    let mut doc = sticker.clone();
    doc.artboard = artboard;
    doc.layers.clear();
    let id = doc.alloc_layer();
    let mut layer = StickerLayer::new(id, &texts::merged_layer_name(cx.editor_name), StickerContent::Picture(PicturePixels::Owned {
        pixels: Arc::new(merged.clone()),
        version: 0,
    }));
    layer.transform = placed([merged.width, merged.height], [0, 0], s, artboard);
    doc.layers.push(layer);
    let top = psd.layers.iter().filter_map(|l| l.lyid).max().unwrap_or(0);
    let mut outcomes = vec![LayerOutcome::New(id)];
    outcomes.extend(sticker.layers.iter().map(|l| LayerOutcome::Deleted(l.id)));
    let toast = (!forcing.is_empty() || !extra.is_empty()).then(|| texts::merged_reason(forcing, extra, cx.editor_name));
    Ok(LinkImport {
        sticker: doc,
        mode: LinkMode::Merged,
        scale: s,
        layer_map: Vec::new(),
        next_lyid: link.next_lyid.max(top.saturating_add(1)).max(1),
        editor_native: psd.editor_native(),
        ids_lost: None,
        outcomes,
        toast,
        profile_note: psd.profile_note().map(|d| texts::profile_note(cx.file_name, d)),
        log: Vec::new(),
    })
}

/// Layers mode (5.4): the table row by row after the matching of 13.2.
fn layers(psd: &PsdDoc, sticker: &StickerDoc, link: &StickerLink, cx: &ImportContext<'_>, s: f32, artboard: IRect) -> LinkImport {
    let canvas = IRect::from_size(psd.size[0], psd.size[1]);
    let mut reads: Vec<Read<'_>> = psd.layers.iter().map(|l| Read::of(l, canvas)).collect();
    // Duplicate ids: the first occurrence from the bottom keeps the id, later ones are "unknown id".
    let mut seen = HashSet::new();
    for r in &mut reads {
        if let Some(id) = r.id
            && !seen.insert(id)
        {
            r.id = None;
        }
    }
    let map = &link.layer_map;
    let (assign, lost) = match_layers(&reads, map);
    let mut next_lyid = link.next_lyid.max(seen.iter().max().map_or(1, |m| m.saturating_add(1))).max(1);
    let mut doc = sticker.clone();
    doc.artboard = artboard;
    doc.layers.clear();
    let mut new_map = Vec::with_capacity(reads.len());
    let mut outcomes = Vec::with_capacity(reads.len());
    let mut kept: HashSet<StickerLayerId> = HashSet::new();
    for (k, r) in reads.iter().enumerate() {
        let bottom = doc.layers.is_empty();
        let lyid = r.id.unwrap_or_else(|| {
            let id = next_lyid;
            next_lyid += 1;
            id
        });
        let matched = assign[k].map(|(i, by_name)| (&map[i], by_name));
        // The placeholder of an empty sticker that is still empty (or only moved) stays out of the sticker.
        if let Some((e, false)) = matched
            && e.layer == PLACEHOLDER_LAYER
            && matches!(change(r, e), Change::Untouched | Change::Moved)
        {
            new_map.push(r.entry(lyid, PLACEHOLDER_LAYER));
            outcomes.push(LayerOutcome::Placeholder);
            continue;
        }
        let old = matched.filter(|(e, _)| !kept.contains(&e.layer)).and_then(|(e, by_name)| Some((e, by_name, sticker.layer(e.layer)?)));
        let layer = match old {
            Some((e, by_name, old)) => {
                let mut l = old.clone();
                let mut ch = if by_name { Change::Pixels } else { change(r, e) };
                // check-P5c: the map entry describes the layer as it was at the sync. A layer changed in WTE since
                // then (an undo of an update, a conflict answered with "Take <editor>'s version") takes the file's
                // pixels; one only moved in WTE goes back to the place the file has for it.
                if ch != Change::Pixels
                    && let Some(w) = e.wte
                    && let Some(&(_, now)) = cx.layer_hashes.iter().find(|x| x.0 == l.id)
                {
                    let tl = Vec2::new(sticker.artboard.x0 as f32, sticker.artboard.y0 as f32);
                    if now != w.hash {
                        ch = Change::Pixels;
                    } else if (l.transform.translate - tl).to_array() != w.at {
                        l.transform.translate = tl + Vec2::from_array(w.at);
                    }
                }
                let delta = (Vec2::new(r.origin[0] as f32, r.origin[1] as f32) - Vec2::new(e.origin[0] as f32, e.origin[1] as f32)) / s;
                match ch {
                    Change::Untouched => outcomes.push(LayerOutcome::Untouched(l.id)),
                    Change::Moved => {
                        l.transform.translate += delta;
                        outcomes.push(LayerOutcome::Moved(l.id));
                    }
                    Change::Mask => {
                        let mut moved = l.clone();
                        moved.transform.translate += delta;
                        if replace_mask(&mut moved, r, s, artboard, cx.blob_sizes) {
                            l = moved;
                            outcomes.push(LayerOutcome::MaskChanged { layer: l.id, kept: true });
                        } else {
                            become_picture(&mut l, r, s, artboard);
                            outcomes.push(LayerOutcome::MaskChanged { layer: l.id, kept: false });
                        }
                    }
                    Change::Pixels => {
                        become_picture(&mut l, r, s, artboard);
                        outcomes.push(LayerOutcome::PixelsChanged(l.id));
                    }
                }
                l
            }
            None => {
                let id = doc.alloc_layer();
                let mut l = StickerLayer::new(id, &r.layer.name, StickerContent::Picture(PicturePixels::Blob(BlobId(0))));
                become_picture(&mut l, r, s, artboard);
                outcomes.push(LayerOutcome::New(id));
                l
            }
        };
        let mut layer = layer;
        take_props(&mut layer, r.layer, bottom);
        kept.insert(layer.id);
        new_map.push(r.entry(lyid, layer.id));
        doc.layers.push(layer);
    }
    outcomes.extend(sticker.layers.iter().filter(|l| !kept.contains(&l.id)).map(|l| LayerOutcome::Deleted(l.id)));
    let mut log = Vec::new();
    if let Some(rule) = lost {
        let line = texts::ids_not_kept(cx.file_name, rule);
        log::info!("{line}");
        log.push(line);
    }
    LinkImport {
        sticker: doc,
        mode: LinkMode::Layers,
        scale: s,
        layer_map: new_map,
        next_lyid,
        editor_native: psd.editor_native(),
        ids_lost: lost,
        outcomes,
        toast: None,
        profile_note: psd.profile_note().map(|d| texts::profile_note(cx.file_name, d)),
        log,
    }
}
