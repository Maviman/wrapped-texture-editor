//! The sticker PSD writer (STICKER_PLAN 5.2 and 13.3; ARCHITECTURE 7.9 "Writer").
//!
//! One writer for every outside editor (PhotoCraft, Photoshop, ...): 8-bit RGB, PSD version 1, PackBits. The bytes
//! follow what Photoshop itself writes (13.3): image resources 1005 / 1044 / 1057, the layer info and the
//! layer-and-mask section rounded up to a multiple of 4, Photoshop's 40-byte default blending ranges on every layer,
//! names and blocks padded to 4, a negative layer count when the merged picture has a transparency channel, and a
//! merged picture that is always written and always real (it never depends on an editor setting).
//!
//! Layer pixels are **normalised** before they are written (review A11): the RGB of every pixel with alpha 0 is set
//! to 0 and the rectangle is trimmed to the pixels with alpha > 0 (an empty layer gets an empty rectangle). That is
//! what PhotoCraft's save produces from the same layer, so a layer nobody touched reads back byte for byte the same.
//! Masks are trimmed the same way to the values that differ from the mask's default.
//!
//! The output is a pure function of the input: no time stamps, no thread-count dependence (rows are encoded in
//! parallel and collected in order).

use std::io::Write;

use rayon::prelude::*;

use super::{PsdError, PsdMask};
use crate::blend::blend_row;
use crate::export::packbits::encode_row;
use crate::layer::BlendMode;
use crate::math::{IRect, mul_u8};
use crate::pixel::{CroppedBuf, PixelBuf, PixelView};
use crate::pixel_ops::content_bounds;

/// Largest canvas side of a version-1 PSD, and the widest layer or mask row the writer accepts (a PackBits row of
/// this width always fits the 16-bit row count).
pub const MAX_SIDE: u32 = 30_000;
/// The program name written into image resource 1057 (writer and reader name).
pub const WRITER_NAME: &str = "Wrapped Texture Editor";

/// Channel ids of a layer in file order: transparency, red, green, blue (the user mask -2 follows when present).
const LAYER_CHANNELS: [i16; 4] = [-1, 0, 1, 2];
/// Byte offset inside an RGBA pixel for each entry of `LAYER_CHANNELS`.
const LAYER_CHANNEL_OFFSETS: [usize; 4] = [3, 0, 1, 2];
/// Photoshop's default blending ranges: the composite gray pair and one pair per colour channel and one more, 5
/// pairs of `00 00 FF FF 00 00 FF FF` whatever the channel count (13.3 rule 4).
const DEFAULT_RANGES: [u8; 40] = {
    let mut r = [0u8; 40];
    let mut i = 0;
    while i < 40 {
        r[i + 2] = 0xFF;
        r[i + 3] = 0xFF;
        r[i + 6] = 0xFF;
        r[i + 7] = 0xFF;
        i += 8;
    }
    r
};

/// One layer of a sticker PSD, bottom to top in [`StickerPsd::layers`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickerPsdLayer {
    /// Written as `luni` (full name) and as the Pascal name (Windows-1252, `?` for other characters).
    pub name: String,
    /// The layer id (`lyid`), at least 1 and unique in the file.
    pub lyid: u32,
    /// Straight RGBA8 in canvas pixels; may reach outside the canvas (the part outside is kept, as Photoshop does).
    /// Normalised by the writer (zero RGB under alpha 0, trimmed to alpha > 0).
    pub pixels: CroppedBuf,
    /// The user mask (channel -2) in canvas pixels; `disabled` is written as mask flags bit 1. `rendered` is not
    /// written (a WTE mask is always a pixel mask). Trimmed by the writer to the values that differ from `default`.
    pub mask: Option<PsdMask>,
    /// Clipped to the layer below. The bottom layer is always written unclipped (it has nothing to clip to).
    pub clipped: bool,
    pub visible: bool,
    pub opacity: u8,
    pub blend: BlendMode,
}

/// What [`write_sticker`] writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickerPsd {
    /// Canvas width and height, 1..=30000 each.
    pub size: [u32; 2],
    /// Layers bottom to top.
    pub layers: Vec<StickerPsdLayer>,
    /// Image resource 1044, the document's id seed: the largest `lyid` ever used for this link (`next_lyid - 1`).
    /// The writer writes at least the largest `lyid` of `layers`, so an editor numbers its new layers above them.
    pub id_seed: u32,
}

/// `pixels` normalised as the writer writes them (and as PhotoCraft's save leaves them): the RGB of every pixel
/// with alpha 0 set to 0, the rectangle trimmed to the pixels with alpha > 0; `CroppedBuf::empty()` when none is.
/// The link's comparison rule (STICKER_PLAN 5.4) normalises a PSD layer the same way before comparing it.
pub fn normalise(pixels: &CroppedBuf) -> CroppedBuf {
    let Some(bounds) = content_bounds(pixels.view()) else { return CroppedBuf::empty() };
    let mut out = CroppedBuf::new(bounds);
    let w = bounds.width() as usize * 4;
    let view = pixels.view();
    for (y, dst) in (bounds.y0..bounds.y1).zip(out.data.chunks_exact_mut(w.max(1))) {
        let row = view.row(y).unwrap_or(&[]);
        let start = (bounds.x0 - pixels.rect.x0) as usize * 4;
        dst.copy_from_slice(&row[start..start + w]);
        for px in dst.as_chunks_mut::<4>().0 {
            if px[3] == 0 {
                *px = [0; 4];
            }
        }
    }
    out
}

/// The mask's rectangle trimmed to the values that differ from its default, with those values (row by row).
/// `IRect::EMPTY` and no values when every value equals the default.
fn trim_mask(mask: &PsdMask) -> (IRect, Vec<u8>) {
    let (w, h) = (mask.pixels.width as usize, mask.pixels.height as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..h {
        let row = &mask.pixels.data[y * w..(y + 1) * w];
        if let Some(first) = row.iter().position(|&v| v != mask.default) {
            let last = row.iter().rposition(|&v| v != mask.default).unwrap_or(first);
            x0 = x0.min(first);
            x1 = x1.max(last + 1);
            y0 = y0.min(y);
            y1 = y + 1;
        }
    }
    if x0 == usize::MAX {
        return (IRect::EMPTY, Vec::new());
    }
    let mut values = Vec::with_capacity((x1 - x0) * (y1 - y0));
    for y in y0..y1 {
        values.extend_from_slice(&mask.pixels.data[y * w + x0..y * w + x1]);
    }
    let r = mask.rect;
    (IRect::new(r.x0 + x0 as i32, r.y0 + y0 as i32, r.x0 + x1 as i32, r.y0 + y1 as i32), values)
}

/// One layer after normalisation, with what the record needs.
struct Prepared {
    pixels: CroppedBuf,
    /// The trimmed mask rectangle and values, the default and the flags byte.
    mask: Option<(IRect, Vec<u8>, u8, u8)>,
    clipped: bool,
}

/// Checks the input and normalises every layer.
fn prepare(psd: &StickerPsd) -> Result<Vec<Prepared>, PsdError> {
    let [w, h] = psd.size;
    if w == 0 || h == 0 {
        return Err(PsdError::Invalid("the canvas has no pixels"));
    }
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(PsdError::TooLarge { width: w, height: h });
    }
    if psd.layers.len() > i16::MAX as usize {
        return Err(PsdError::Invalid("too many layers for a PSD file"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for layer in &psd.layers {
        if layer.lyid == 0 || !ids.insert(layer.lyid) {
            return Err(PsdError::Invalid("layer ids must be unique and at least 1"));
        }
        if layer.pixels.data.len() as u64 != layer.pixels.rect.area() * 4 {
            return Err(PsdError::Invalid("layer pixels do not match their rectangle"));
        }
        if let Some(m) = &layer.mask
            && (m.pixels.width != m.rect.width()
                || m.pixels.height != m.rect.height()
                || m.pixels.data.len() as u64 != m.rect.area())
        {
            return Err(PsdError::Invalid("mask values do not match their rectangle"));
        }
        if layer.mask.as_ref().is_some_and(|m| m.default != 0 && m.default != 255) {
            return Err(PsdError::Invalid("a mask's default must be 0 or 255"));
        }
    }
    let prepared: Vec<Prepared> = psd
        .layers
        .par_iter()
        .enumerate()
        .map(|(i, layer)| Prepared {
            pixels: normalise(&layer.pixels),
            mask: layer.mask.as_ref().map(|m| {
                let (rect, values) = trim_mask(m);
                (rect, values, m.default, if m.disabled { 0x02 } else { 0 })
            }),
            clipped: layer.clipped && i > 0,
        })
        .collect();
    for p in &prepared {
        let mask_w = p.mask.as_ref().map_or(0, |m| m.0.width());
        if p.pixels.rect.width() > MAX_SIDE || mask_w > MAX_SIDE {
            return Err(PsdError::Invalid("a layer or mask is wider than 30000 pixels"));
        }
    }
    Ok(prepared)
}

/// The merged picture of exactly the layers [`write_sticker`] writes (straight RGBA8, canvas size): WTE's blend
/// kernels, layer opacity, the user mask (unless disabled) and clipping to the nearest unclipped layer below (a
/// hidden base hides the layers clipped to it, as in the sticker render, ARCHITECTURE 6.10). Layer styles are not
/// part of a sticker PSD. The file stores this picture matted on white.
pub fn composite(psd: &StickerPsd) -> Result<PixelBuf, PsdError> {
    let prepared = prepare(psd)?;
    Ok(composite_prepared(psd, &prepared))
}

fn composite_prepared(psd: &StickerPsd, prepared: &[Prepared]) -> PixelBuf {
    let [w, h] = psd.size;
    let mut out = PixelBuf::new(w, h);
    let canvas = out.rect();
    let stride = w as usize * 4;
    // The current base: its visibility and its coverage (alpha times mask, before opacity) over its canvas rect.
    let mut base: Option<(bool, CroppedBuf)> = None;
    for (layer, p) in psd.layers.iter().zip(prepared) {
        let rect = p.pixels.rect.intersect(&canvas);
        let rect = if rect.is_empty() { IRect::EMPTY } else { rect };
        let rw = rect.width() as usize;
        let n = rw * 4;
        // The layer over `rect`, its alpha multiplied by the mask (rows in parallel; the same bytes on any thread
        // count).
        let mut drawn = CroppedBuf::new(rect);
        let mask = p.mask.as_ref().filter(|m| m.3 & 0x02 == 0);
        let src = p.pixels.view();
        drawn.data.par_chunks_mut(n.max(1)).enumerate().for_each(|(i, row)| {
            let y = rect.y0 + i as i32;
            let s = src.row(y).unwrap_or(&[]);
            let start = (rect.x0 - p.pixels.rect.x0) as usize * 4;
            row.copy_from_slice(&s[start..start + n]);
            if let Some((mrect, values, default, _)) = mask {
                let inside_y = (mrect.y0..mrect.y1).contains(&y);
                for (k, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let x = rect.x0 + k as i32;
                    let m = if inside_y && (mrect.x0..mrect.x1).contains(&x) {
                        values[(y - mrect.y0) as usize * mrect.width() as usize + (x - mrect.x0) as usize]
                    } else {
                        *default
                    };
                    px[3] = mul_u8(px[3], m);
                }
            }
        });
        if p.clipped {
            let Some((base_visible, cut)) = &base else { continue };
            if !*base_visible {
                continue;
            }
            let cut = cut.view();
            drawn.data.par_chunks_mut(n.max(1)).enumerate().for_each(|(i, row)| {
                let y = rect.y0 + i as i32;
                for (k, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    px[3] = mul_u8(px[3], cut.get(rect.x0 + k as i32, y)[3]);
                }
            });
        }
        if layer.visible && layer.opacity > 0 && !rect.is_empty() {
            for (row, y) in drawn.data.chunks_exact(n).zip(rect.y0..rect.y1) {
                let d = y as usize * stride + rect.x0 as usize * 4;
                blend_row(&mut out.data[d..d + n], row, layer.opacity, layer.blend);
            }
        }
        if !p.clipped {
            base = Some((layer.visible, drawn));
        }
    }
    out
}

/// Windows-1252 code points of the bytes 0x80..=0x9F that are defined (0 = undefined).
const CP1252_HIGH: [u16; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0, 0,
    0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
];

/// One character in Windows-1252, `?` when it has no byte there (also control characters).
fn cp1252(c: char) -> u8 {
    let u = c as u32;
    match u {
        0x20..=0x7E | 0xA0..=0xFF => u as u8,
        _ => CP1252_HIGH.iter().position(|&v| v != 0 && v as u32 == u).map_or(b'?', |i| 0x80 + i as u8),
    }
}

/// The Pascal name: length byte and at most 255 Windows-1252 bytes, padded with zeros so that the length byte and
/// the name together are a multiple of 4.
fn pascal_name(name: &str) -> Vec<u8> {
    let bytes: Vec<u8> = name.chars().take(255).map(cp1252).collect();
    let mut out = Vec::with_capacity(bytes.len() + 4);
    out.push(bytes.len() as u8);
    out.extend_from_slice(&bytes);
    out.resize(out.len().div_ceil(4) * 4, 0);
    out
}

/// A tagged block `8BIM key len data`, `data` padded with zeros to a multiple of 4 (the length counts the padding,
/// as in Photoshop's layer blocks).
fn tagged_block(out: &mut Vec<u8>, key: &[u8; 4], data: &[u8]) {
    let len = data.len().div_ceil(4) * 4;
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(key);
    out.extend_from_slice(&(len as u32).to_be_bytes());
    out.extend_from_slice(data);
    out.resize(out.len() + len - data.len(), 0);
}

/// A Unicode string of the format: count of UTF-16 units, then the units big-endian (no terminator).
fn unicode_string(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().collect();
    out.extend_from_slice(&(units.len() as u32).to_be_bytes());
    for u in units {
        out.extend_from_slice(&u.to_be_bytes());
    }
}

/// One image resource: `8BIM`, id, empty Pascal name (2 zero bytes), length, data padded to even.
fn resource(out: &mut Vec<u8>, id: u16, data: &[u8]) {
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

/// The image resources 1005 (72 dpi), 1044 (id seed) and 1057 (version info, real merged data).
fn image_resources(id_seed: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(160);
    let mut res = Vec::with_capacity(16);
    for _ in 0..2 {
        res.extend_from_slice(&[0x00, 0x48, 0x00, 0x00]); // 72.0 as 16.16 fixed
        res.extend_from_slice(&1u16.to_be_bytes()); // pixels per inch
        res.extend_from_slice(&1u16.to_be_bytes()); // shown in inches
    }
    resource(&mut out, 1005, &res);
    resource(&mut out, 1044, &id_seed.to_be_bytes());
    let mut v = Vec::with_capacity(112);
    v.extend_from_slice(&1u32.to_be_bytes());
    v.push(1); // hasRealMergedData
    unicode_string(&mut v, WRITER_NAME);
    unicode_string(&mut v, WRITER_NAME);
    v.extend_from_slice(&1u32.to_be_bytes());
    resource(&mut out, 1057, &v);
    out
}

/// One channel's data: compression 1 with the row counts first and the PackBits rows, or compression 0 and no bytes
/// for an empty rectangle. `plane_of(row, out)` fills one row's samples.
fn rle_channel(rows: usize, plane_of: impl Fn(usize, &mut Vec<u8>) + Sync) -> Vec<u8> {
    if rows == 0 {
        return vec![0, 0];
    }
    let encoded: Vec<Vec<u8>> = (0..rows)
        .into_par_iter()
        .map_init(Vec::new, |plane, y| {
            plane.clear();
            plane_of(y, plane);
            let mut out = Vec::with_capacity(plane.len() / 4 + 8);
            encode_row(plane, &mut out);
            out
        })
        .collect();
    let mut out = Vec::with_capacity(2 + 2 * rows + encoded.iter().map(Vec::len).sum::<usize>());
    out.extend_from_slice(&1u16.to_be_bytes());
    for row in &encoded {
        out.extend_from_slice(&(row.len() as u16).to_be_bytes());
    }
    for row in &encoded {
        out.extend_from_slice(row);
    }
    out
}

/// The four (or five) channels of one layer in file order.
fn layer_channels(p: &Prepared) -> Vec<(i16, Vec<u8>)> {
    let view: PixelView<'_> = p.pixels.view();
    let rect = p.pixels.rect;
    let rows = if rect.is_empty() { 0 } else { rect.height() as usize };
    let mut out: Vec<(i16, Vec<u8>)> = LAYER_CHANNELS
        .iter()
        .zip(LAYER_CHANNEL_OFFSETS)
        .map(|(&id, offset)| {
            let data = rle_channel(rows, |y, plane| {
                let row = view.row(rect.y0 + y as i32).unwrap_or(&[]);
                plane.extend(row.as_chunks::<4>().0.iter().map(|px| px[offset]));
            });
            (id, data)
        })
        .collect();
    if let Some((mrect, values, _, _)) = &p.mask {
        let mw = mrect.width() as usize;
        let rows = if mrect.is_empty() { 0 } else { mrect.height() as usize };
        out.push((-2, rle_channel(rows, |y, plane| plane.extend_from_slice(&values[y * mw..(y + 1) * mw]))));
    }
    out
}

/// The record of one layer (everything before its channel data).
fn layer_record(layer: &StickerPsdLayer, p: &Prepared, channels: &[(i16, Vec<u8>)]) -> Vec<u8> {
    let mut extra = Vec::with_capacity(128);
    match &p.mask {
        Some((r, _, default, flags)) => {
            extra.extend_from_slice(&20u32.to_be_bytes());
            for v in [r.y0, r.x0, r.y1, r.x1] {
                extra.extend_from_slice(&v.to_be_bytes());
            }
            extra.extend_from_slice(&[*default, *flags, 0, 0]);
        }
        None => extra.extend_from_slice(&0u32.to_be_bytes()),
    }
    extra.extend_from_slice(&(DEFAULT_RANGES.len() as u32).to_be_bytes());
    extra.extend_from_slice(&DEFAULT_RANGES);
    extra.extend_from_slice(&pascal_name(&layer.name));
    let mut luni = Vec::new();
    unicode_string(&mut luni, &layer.name);
    tagged_block(&mut extra, b"luni", &luni);
    tagged_block(&mut extra, b"lyid", &layer.lyid.to_be_bytes());

    let r = p.pixels.rect;
    let mut rec = Vec::with_capacity(64 + extra.len());
    for v in [r.y0, r.x0, r.y1, r.x1] {
        rec.extend_from_slice(&v.to_be_bytes());
    }
    rec.extend_from_slice(&(channels.len() as u16).to_be_bytes());
    for (id, data) in channels {
        rec.extend_from_slice(&id.to_be_bytes());
        rec.extend_from_slice(&(data.len() as u32).to_be_bytes());
    }
    rec.extend_from_slice(b"8BIM");
    rec.extend_from_slice(&layer.blend.psd_key());
    rec.push(layer.opacity);
    rec.push(p.clipped as u8);
    rec.push(0x08 | if layer.visible { 0 } else { 0x02 });
    rec.push(0);
    rec.extend_from_slice(&(extra.len() as u32).to_be_bytes());
    rec.extend_from_slice(&extra);
    rec
}

/// The merged picture's section: compression 1, every row count of every channel first, then the rows; 3 channels
/// when opaque, else 4 with the transparency last and the colours matted on white.
fn merged_section(merged: &PixelBuf, channels: usize) -> Vec<u8> {
    let (w, h) = (merged.width as usize, merged.height as usize);
    let matte = channels == 4;
    let rows: Vec<Vec<Vec<u8>>> = (0..channels)
        .map(|c| {
            (0..h)
                .into_par_iter()
                .map_init(Vec::new, |plane, y| {
                    plane.clear();
                    plane.extend(merged.row(y as u32).as_chunks::<4>().0.iter().map(|px| {
                        if c == 3 || !matte {
                            px[c]
                        } else {
                            let a = px[3] as u32;
                            ((px[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8
                        }
                    }));
                    let mut out = Vec::with_capacity(w / 4 + 8);
                    encode_row(plane, &mut out);
                    out
                })
                .collect()
        })
        .collect();
    let body: usize = rows.iter().flatten().map(Vec::len).sum();
    let mut out = Vec::with_capacity(2 + 2 * channels * h + body);
    out.extend_from_slice(&1u16.to_be_bytes());
    for row in rows.iter().flatten() {
        out.extend_from_slice(&(row.len() as u16).to_be_bytes());
    }
    for row in rows.iter().flatten() {
        out.extend_from_slice(row);
    }
    out
}

/// Writes a sticker as a layered PSD (STICKER_PLAN 5.2, 13.3; ARCHITECTURE 7.9 "Writer").
///
/// Errors (nothing is written): `Invalid` for a canvas without pixels, more than 32767 layers, a layer id of 0 or
/// used twice, pixels or mask values that do not match their rectangle, a mask default other than 0 or 255, a layer
/// or mask row wider than 30000, or a file over 4 GB; `TooLarge` for a canvas side above 30000. `Io` when `w` fails.
pub fn write_sticker<W: Write>(mut w: W, psd: &StickerPsd) -> Result<(), PsdError> {
    let bytes = write_sticker_to_vec(psd)?;
    w.write_all(&bytes).and_then(|()| w.flush()).map_err(PsdError::io)
}

/// [`write_sticker`] into a new vector.
pub fn write_sticker_to_vec(psd: &StickerPsd) -> Result<Vec<u8>, PsdError> {
    let prepared = prepare(psd)?;
    let merged = composite_prepared(psd, &prepared);
    let transparent = merged.data.as_chunks::<4>().0.iter().any(|px| px[3] != 255);
    let merged_channels = if transparent { 4 } else { 3 };

    // Layer info: count, records, channel data, padded to a multiple of 4.
    let encoded: Vec<Vec<(i16, Vec<u8>)>> = prepared.iter().map(layer_channels).collect();
    let mut info = Vec::new();
    if !psd.layers.is_empty() {
        let count = psd.layers.len() as i16;
        info.extend_from_slice(&(if transparent { -count } else { count }).to_be_bytes());
        for ((layer, p), channels) in psd.layers.iter().zip(&prepared).zip(&encoded) {
            info.extend_from_slice(&layer_record(layer, p, channels));
        }
        for channels in &encoded {
            for (_, data) in channels {
                info.extend_from_slice(data);
            }
        }
        info.resize(info.len().div_ceil(4) * 4, 0);
    }
    // The section: layer info length and data, global layer mask info of length 0, no further blocks.
    let section_len = 4 + info.len() as u64 + 4;
    if section_len > u32::MAX as u64 {
        return Err(PsdError::Invalid("the PSD file would be larger than 4 GB"));
    }
    let id_seed = psd.layers.iter().map(|l| l.lyid).max().unwrap_or(0).max(psd.id_seed);
    let resources = image_resources(id_seed);
    let merged_bytes = merged_section(&merged, merged_channels);
    let total = 26 + 4 + 4 + resources.len() as u64 + 4 + section_len + merged_bytes.len() as u64;
    if total > u32::MAX as u64 {
        return Err(PsdError::Invalid("the PSD file would be larger than 4 GB"));
    }

    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(&(merged_channels as u16).to_be_bytes());
    out.extend_from_slice(&psd.size[1].to_be_bytes());
    out.extend_from_slice(&psd.size[0].to_be_bytes());
    out.extend_from_slice(&8u16.to_be_bytes());
    out.extend_from_slice(&3u16.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes()); // colour mode data
    out.extend_from_slice(&(resources.len() as u32).to_be_bytes());
    out.extend_from_slice(&resources);
    out.extend_from_slice(&(section_len as u32).to_be_bytes());
    out.extend_from_slice(&(info.len() as u32).to_be_bytes());
    out.extend_from_slice(&info);
    out.extend_from_slice(&0u32.to_be_bytes()); // global layer mask info
    out.extend_from_slice(&merged_bytes);
    Ok(out)
}
