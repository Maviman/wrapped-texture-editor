//! Linked texture paint layers (STICKER_PLAN 5.9, 13.9 "P12"; stickers P12; ARCHITECTURE 5.28 "link", 6.13).
//!
//! A raster paint layer can be edited in an outside editor the way a sticker is: WTE writes a one-layer PSD at the
//! texture set's size (the layer's pixels, straight and normalised; opacity 255, Normal: the layer's own opacity and
//! blend stay in WTE and are not sent), the editor saves it, and the layer takes the file's picture back as one undo
//! step "Update Linked Layer" (`Document::replace_layer_pixels`).
//!
//! What comes back ([`paint_source`]): a file whose canvas is not the texture's size is refused (the sentence says to
//! undo the size change); with nothing forcing the merged path and at most one visible pixel layer, that layer on a
//! transparent canvas (its mask and opacity applied); otherwise the file's merged picture, which must be real merged
//! data (else "needs a merged picture", worded for the editor). The state table, the stale rule, the writes and the
//! watcher are the sticker link's (5.5-5.7): `wte_changed` = the CRC-32 of the layer's normalised pixels differs from
//! [`PaintLink::synced_pixels`].
//!
//! Choice (P12): the plan's `synced_pixels` is "the layer's version"; layer versions are not kept across a save and
//! a load, so the link stores the CRC-32 of the normalised pixels instead ([`normalised_crc`]): the same after a
//! reload, and cheap to keep current (the document caches it by layer version).

use crate::ids::LayerId;
use crate::layer::BlendMode;
use crate::math::IRect;
use crate::pixel::{CroppedBuf, PixelBuf};
use crate::psd::{PsdDoc, PsdSection, StickerPsd, StickerPsdLayer};

use super::{FileStamp, LinkFile, LinkProblem, smart_forces};

/// The undo label of an applied update of a linked paint layer (5.9).
pub const PAINT_UPDATE_LABEL: &str = "Update Linked Layer";

/// The largest texture side a layer link takes (the reader decodes up to 8192 a side, 5.2).
pub const MAX_PAINT_LINK_SIDE: u32 = 8192;

/// The message for a texture set over [`MAX_PAINT_LINK_SIDE`] (5.9).
pub const TOO_LARGE_TO_LINK: &str = "This texture is too large to link.";

/// The link of one raster paint layer (5.9). Kept in `Document` outside the history, like a sticker link (R6).
#[derive(Clone, Debug, PartialEq)]
pub struct PaintLink {
    pub file: LinkFile,
    /// The editor's key (`"photocraft"`, `"photoshop"`, `"other"`), set at every "Edit Layer in" (13.1).
    pub editor: String,
    /// The file content WTE last wrote or applied.
    pub synced: FileStamp,
    /// CRC-32 of the layer's normalised pixels ([`normalised_crc`]) that correspond to `synced`.
    pub synced_pixels: u32,
    /// The file held more than WTE's one plain layer when it was last applied (editor layers, effects, text): a
    /// rewrite from WTE flattens them into one picture layer (the "Send first?" question says so, 5.5).
    pub editor_layers: bool,
    /// Why the file with this content could not be applied (not saved).
    pub problem: Option<LinkProblem>,
}

/// Why a file cannot be applied to a paint layer ([`paint_source`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaintProblem {
    /// The canvas is not the texture's size ("Undo the size change in <editor>.").
    SizeMismatch { size: [u32; 2], texture: [u32; 2] },
    /// The merged picture is needed but the file has none (MaxCompat off).
    NeedsMerged,
    /// The pixels were not decoded (over the reader's limits).
    TooLarge { width: u32, height: u32 },
}

impl PaintProblem {
    /// The problem recorded for the file content `crc32`.
    pub fn into_link_problem(self, crc32: u32) -> LinkProblem {
        match self {
            PaintProblem::SizeMismatch { size, texture } => LinkProblem::SizeMismatch { crc32, size, texture },
            PaintProblem::NeedsMerged => LinkProblem::NeedsMerged { crc32 },
            PaintProblem::TooLarge { width, height } => LinkProblem::TooLarge { crc32, width, height },
        }
    }
}

/// The picture a file gives a paint layer, and how it was chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaintPicture {
    /// Texture-sized, straight RGBA, normalised (zero RGB under alpha 0).
    pub pixels: PixelBuf,
    /// Taken from the merged picture (not from the one layer).
    pub merged: bool,
    /// The file holds more than one plain layer, a forcing feature or the editor's own layer kinds.
    pub editor_layers: bool,
}

/// The file name of a paint layer's link file: `<sanitised layer name>_l<id>.psd` (5.3), the rules of
/// `link_file_name` ("layer" for an empty name).
pub fn paint_link_file_name(layer_name: &str, id: LayerId) -> String {
    let mut name: String = layer_name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .take(40)
        .collect();
    name = name.trim().trim_end_matches('.').to_owned();
    if name.is_empty() {
        name = "layer".to_owned();
    }
    format!("{name}_l{}.psd", id.0)
}

/// The archive entry of a paint layer's link file inside the project: `links/l<id:05>.psd`.
pub fn paint_archive_entry(id: LayerId) -> String {
    format!("links/l{:05}.psd", id.0)
}

/// Sets the RGB of every pixel with alpha 0 to 0 (the PSD normalisation of 5.2).
pub fn normalise_in_place(data: &mut [u8]) {
    for px in data.as_chunks_mut::<4>().0 {
        if px[3] == 0 {
            *px = [0; 4];
        }
    }
}

/// CRC-32 of a paint layer's pixels at `width x height`, normalised (zero RGB under alpha 0) and row by row. `buf` is
/// the layer's raster: `width x height`, or `0 x 0` for a layer that was never painted (all transparent: the CRC of
/// zeros, without allocating the texture).
pub fn normalised_crc(buf: &PixelBuf, width: u32, height: u32) -> u32 {
    let mut h = crc32fast::Hasher::new();
    let row_len = width as usize * 4;
    if buf.width != width || buf.height != height || buf.data.len() != row_len * height as usize {
        let zeros = vec![0u8; row_len];
        for _ in 0..height {
            h.update(&zeros);
        }
        return h.finalize();
    }
    let mut row = vec![0u8; row_len];
    for y in 0..height {
        row.copy_from_slice(buf.row(y));
        normalise_in_place(&mut row);
        h.update(&row);
    }
    h.finalize()
}

/// The one-layer PSD of a paint layer (5.9): the layer's pixels (an unallocated layer: an empty layer) at the
/// texture's size, opacity 255, Normal, visible, `lyid` 1; the writer normalises and trims them and writes the same
/// picture as the merged image.
pub fn layer_psd(name: &str, pixels: &PixelBuf, width: u32, height: u32) -> StickerPsd {
    let full = pixels.width == width && pixels.height == height && pixels.data.len() == width as usize * height as usize * 4;
    let layer_pixels = if full && width > 0 && height > 0 {
        CroppedBuf { rect: IRect::new(0, 0, width as i32, height as i32), data: pixels.data.clone() }
    } else {
        CroppedBuf::empty()
    };
    StickerPsd {
        size: [width, height],
        layers: vec![StickerPsdLayer {
            name: name.to_owned(),
            lyid: 1,
            pixels: layer_pixels,
            mask: None,
            clipped: false,
            visible: true,
            opacity: 255,
            blend: BlendMode::Normal,
        }],
        id_seed: 1,
    }
}

/// The layers of `psd` that show pixels (visible ordinary layers with at least one decoded pixel).
fn visible_pixel_layers(psd: &PsdDoc) -> Vec<usize> {
    psd.layers
        .iter()
        .enumerate()
        .filter(|(_, l)| l.visible && l.section == PsdSection::Layer && l.pixels.as_ref().is_some_and(|p| !p.is_empty()))
        .map(|(i, _)| i)
        .collect()
}

/// What a file gives a paint layer of a `texture` sized set (5.9): refused when its canvas has another size; with
/// nothing forcing and at most one visible pixel layer (not clipped onto a layer below), that layer on a transparent
/// canvas with its mask and opacity applied (a file whose layers are all hidden or empty gives a transparent
/// picture); otherwise the merged picture, which must be real merged data. `editor_key` decides whether smart
/// objects force the merged path (13.2).
pub fn paint_source(psd: &PsdDoc, texture: [u32; 2], editor_key: &str) -> Result<PaintPicture, PaintProblem> {
    if psd.size != texture {
        return Err(PaintProblem::SizeMismatch { size: psd.size, texture });
    }
    if psd.too_large {
        return Err(PaintProblem::TooLarge { width: psd.size[0], height: psd.size[1] });
    }
    let forcing = psd.forcing(smart_forces(editor_key));
    let shown = visible_pixel_layers(psd);
    let editor_layers = !forcing.is_empty() || psd.editor_native() || psd.layers.len() > 1;
    let [w, h] = texture;
    let single = match shown.as_slice() {
        [] => true,
        [i] => !psd.layers[*i].clipped || !psd.layers[..*i].iter().any(|l| l.section == PsdSection::Layer),
        _ => false,
    };
    if forcing.is_empty() && !psd.is_flat() && single {
        let mut out = PixelBuf::new(w, h);
        if let Some(&i) = shown.first() {
            let layer = &psd.layers[i];
            if let Some(src) = &layer.pixels {
                let mask = layer.mask.as_ref().filter(|m| !m.disabled);
                let rect = src.rect.intersect(&IRect::new(0, 0, w as i32, h as i32));
                let opacity = u32::from(layer.opacity);
                let n = rect.width() as usize * 4;
                for y in rect.y0..rect.y1 {
                    let Some(row) = src.view().row(y) else { continue };
                    let from = (rect.x0 - src.rect.x0) as usize * 4;
                    let to = rect.x0 as usize * 4;
                    let dst = &mut out.row_mut(y as u32)[to..to + n];
                    dst.copy_from_slice(&row[from..from + n]);
                    if opacity == 255 && mask.is_none() {
                        continue;
                    }
                    for (px, x) in dst.as_chunks_mut::<4>().0.iter_mut().zip(rect.x0..) {
                        let m = mask.map_or(255, |m| u32::from(m.value(x, y)));
                        px[3] = ((u32::from(px[3]) * opacity * m + 255 * 255 / 2) / (255 * 255)) as u8;
                    }
                }
            }
        }
        normalise_in_place(&mut out.data);
        return Ok(PaintPicture { pixels: out, merged: false, editor_layers });
    }
    match &psd.merged {
        Some(merged) if psd.has_real_merged && merged.width == w && merged.height == h => {
            let mut pixels = merged.clone();
            normalise_in_place(&mut pixels.data);
            Ok(PaintPicture { pixels, merged: true, editor_layers })
        }
        _ => Err(PaintProblem::NeedsMerged),
    }
}
