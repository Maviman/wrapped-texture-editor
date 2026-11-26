//! The PSD / PSB reader (STICKER_PLAN 5.2 and 13.2; ARCHITECTURE 7.9 "Reader").
//!
//! The file is walked once, front to back. Every length is checked against the bytes that remain: running out at
//! the top level is [`PsdError::Truncated`] (a half-written file), running out inside a part whose length is known is
//! [`PsdError::Corrupt`]. Pixel decoding validates before it allocates: a PackBits row table must hold at least the
//! two bytes per 128 output bytes that any row needs, and an inflate is refused when its output could not come from
//! its input, so no length in a file can make the reader allocate much more than the file could describe.

use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit;

use super::descriptor;
use super::{
    PsdBlend, PsdColorMode, PsdDoc, PsdError, PsdFeatures, PsdLayer, PsdLimits, PsdMask, PsdRefusal, PsdSection,
};
use crate::export::packbits::decode_row;
use crate::math::IRect;
use crate::pixel::{AlphaBuf, CroppedBuf, PixelBuf};

/// Tagged-block keys whose length is 8 bytes in a PSB (13.2: the 13 of PhotoCraft's list plus `cinf`).
const PSB_LONG_KEYS: [&[u8; 4]; 14] = [
    b"LMsk", b"Lr16", b"Lr32", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2", b"FEid", b"FXid", b"PxSD",
    b"cinf",
];

/// True for a key whose tagged-block length is 8 bytes in a PSB.
pub(crate) fn psb_long_key(key: &[u8; 4]) -> bool {
    PSB_LONG_KEYS.contains(&key)
}

/// Keys of adjustment layers (5.2, 13.2).
const ADJUSTMENT_KEYS: [&[u8; 4]; 19] = [
    b"levl", b"curv", b"hue2", b"hue ", b"brit", b"blnc", b"expA", b"vibA", b"blwh", b"phfl", b"mixr", b"clrL", b"nvrt",
    b"post", b"thrs", b"grdm", b"selc", b"CgEd", b"brst",
];

/// Most channels a header or layer record may name.
const MAX_CHANNELS: u16 = 56;
/// The largest ratio of inflated to deflated bytes a zlib stream can reach (deflate's limit is about 1032:1).
const MAX_INFLATE_RATIO: usize = 1100;

/// Reads a PSD (version 1) or PSB (version 2) file.
///
/// Refuses (with [`PsdError::Unsupported`]) colour modes other than RGB and Grayscale and depths other than 8 and 16;
/// see STICKER_PLAN 13.2 for every rule. Never panics.
pub fn read(bytes: &[u8], limits: &PsdLimits) -> Result<PsdDoc, PsdError> {
    parse(bytes, limits).map(|p| p.doc)
}

/// The result of [`parse`]: the document and, for tests, the file offset of every layer record's flags byte.
pub(crate) struct Parsed {
    pub doc: PsdDoc,
    #[cfg_attr(not(test), allow(dead_code))]
    pub flag_offsets: Vec<usize>,
    /// The file offset where the merged picture's data ended (bytes after it are ignored).
    #[cfg_attr(not(test), allow(dead_code))]
    pub end: usize,
}

#[cfg(test)]
thread_local! {
    /// Tagged-block walks that had to re-synchronise (non-zero bytes skipped, or a scan for the next signature).
    pub(crate) static RESYNCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn note_resync() {
    #[cfg(test)]
    RESYNCS.with(|r| r.set(r.get() + 1));
}

/// A position in the file with the error to return when bytes run out.
#[derive(Clone)]
struct Cur<'a> {
    data: &'a [u8],
    /// Offset of `data[0]` in the whole file.
    base: usize,
    pos: usize,
    psb: bool,
    short: PsdError,
}

impl<'a> Cur<'a> {
    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn file_offset(&self) -> usize {
        self.base + self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], PsdError> {
        if n > self.remaining() {
            return Err(self.short.clone());
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn take_u64(&mut self, n: u64) -> Result<&'a [u8], PsdError> {
        if n > self.remaining() as u64 {
            return Err(self.short.clone());
        }
        self.take(n as usize)
    }

    /// A part of known length; running out inside it is `Corrupt(what)`.
    fn sub(&mut self, n: u64, what: &'static str) -> Result<Cur<'a>, PsdError> {
        let base = self.file_offset();
        let data = self.take_u64(n)?;
        Ok(Cur { data, base, pos: 0, psb: self.psb, short: PsdError::Corrupt(what) })
    }

    fn u8(&mut self) -> Result<u8, PsdError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, PsdError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn i16(&mut self) -> Result<i16, PsdError> {
        Ok(self.u16()? as i16)
    }

    fn u32(&mut self) -> Result<u32, PsdError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> Result<i32, PsdError> {
        Ok(self.u32()? as i32)
    }

    fn u64(&mut self) -> Result<u64, PsdError> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    /// A section length: 4 bytes in a PSD, 8 in a PSB.
    fn len(&mut self) -> Result<u64, PsdError> {
        if self.psb { self.u64() } else { Ok(self.u32()? as u64) }
    }

    fn key4(&mut self) -> Result<[u8; 4], PsdError> {
        let b = self.take(4)?;
        Ok([b[0], b[1], b[2], b[3]])
    }
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn is_signature(b: &[u8]) -> bool {
    b == b"8BIM" || b == b"8B64"
}

/// Header facts every later part needs.
#[derive(Copy, Clone)]
struct Ctx {
    psb: bool,
    mode: PsdColorMode,
    /// Bytes per sample: 1 or 2.
    bps: usize,
    /// Pixels are decoded (the canvas is within the decode limits).
    decode: bool,
    max_decode_pixels: u64,
}

impl Ctx {
    fn colour_channels(&self) -> usize {
        match self.mode {
            PsdColorMode::Rgb => 3,
            PsdColorMode::Grayscale => 1,
        }
    }
}

pub(crate) fn parse(bytes: &[u8], limits: &PsdLimits) -> Result<Parsed, PsdError> {
    let mut c = Cur { data: bytes, base: 0, pos: 0, psb: false, short: PsdError::Truncated };

    // Header (26 bytes).
    let sig = c.take(4)?;
    if sig != b"8BPS" {
        return Err(PsdError::Corrupt("not a PSD file"));
    }
    let version = c.u16()?;
    let psb = match version {
        1 => false,
        2 => true,
        _ => return Err(PsdError::Corrupt("unknown PSD version")),
    };
    c.psb = psb;
    c.take(6)?;
    let channels = c.u16()?;
    let height = c.u32()?;
    let width = c.u32()?;
    let depth = c.u16()?;
    let mode = c.u16()?;
    let mode = match mode {
        3 => PsdColorMode::Rgb,
        1 => PsdColorMode::Grayscale,
        0 => return Err(PsdError::Unsupported(PsdRefusal::Bitmap)),
        2 => return Err(PsdError::Unsupported(PsdRefusal::Indexed)),
        4 => return Err(PsdError::Unsupported(PsdRefusal::Cmyk)),
        7 => return Err(PsdError::Unsupported(PsdRefusal::Multichannel)),
        8 => return Err(PsdError::Unsupported(PsdRefusal::Duotone)),
        9 => return Err(PsdError::Unsupported(PsdRefusal::Lab)),
        m => return Err(PsdError::Unsupported(PsdRefusal::OtherMode(m))),
    };
    let bps = match depth {
        8 => 1,
        16 => 2,
        1 => return Err(PsdError::Unsupported(PsdRefusal::Bitmap)),
        32 => return Err(PsdError::Unsupported(PsdRefusal::Depth32)),
        d => return Err(PsdError::Unsupported(PsdRefusal::OtherDepth(d))),
    };
    if width == 0 || height == 0 {
        return Err(PsdError::Corrupt("zero canvas size"));
    }
    if width > limits.max_side || height > limits.max_side {
        return Err(PsdError::TooLarge { width, height });
    }
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(PsdError::Corrupt("channel count"));
    }
    let decode = width <= limits.max_decode_side
        && height <= limits.max_decode_side
        && width as u64 * height as u64 <= limits.max_decode_pixels;
    let ctx = Ctx { psb, mode, bps, decode, max_decode_pixels: limits.max_decode_pixels };
    if (channels as usize) < ctx.colour_channels() {
        return Err(PsdError::Corrupt("fewer channels than the colour mode needs"));
    }

    // Colour mode data.
    let n = c.u32()?;
    c.take_u64(n as u64)?;

    // Image resources.
    let n = c.u32()?;
    let mut res = c.sub(n as u64, "image resources")?;
    let (real_resource, icc_description) = read_resources(&mut res)?;

    // Layer and mask information.
    let n = c.len()?;
    let mut lm = c.sub(n, "layer and mask information")?;
    let mut main: Option<LayerSet> = None;
    let mut global: Option<LayerSet> = None;
    let mut flag_offsets = Vec::new();
    if lm.remaining() > 0 {
        let n = lm.len()?;
        let mut li = lm.sub(n, "layer info")?;
        if n > 0 {
            main = Some(read_layer_info(&mut li, &ctx)?);
        }
        if lm.remaining() >= 4 {
            let n = lm.u32()?;
            lm.take_u64(n as u64)?;
            for (key, data, base) in tagged_blocks(&mut lm)? {
                let main_empty = main.as_ref().is_none_or(|m| m.layers.is_empty());
                if matches!(&key, b"Lr16" | b"Lr32" | b"Layr") && global.is_none() && main_empty {
                    let mut b = Cur { data, base, pos: 0, psb, short: PsdError::Corrupt("layer block") };
                    if !data.is_empty() {
                        global = Some(read_layer_info(&mut b, &ctx)?);
                    }
                }
            }
        }
    }
    // "0 layers" is counted after Lr16 / Lr32 (13.2): the main layer info of a 16-bit file is empty.
    let set = match (main, global) {
        (Some(m), _) if !m.layers.is_empty() => m,
        (_, Some(g)) => g,
        (m, None) => m.unwrap_or_default(),
    };
    let flat = set.layers.is_empty();
    for l in &set.layers {
        flag_offsets.push(l.flags_offset);
    }
    let too_large = !decode || set.too_large;

    // Merged picture.
    let colour = ctx.colour_channels();
    let transparent = (set.negative || flat) && channels as usize > colour;
    let need = colour + transparent as usize;
    let compression = c.u16()?;
    let merged_planes = read_merged(&mut c, compression, width, height, channels as usize, need, &ctx)?;
    let has_real_merged = flat || real_resource.unwrap_or(true);
    let merged = match merged_planes {
        Some(planes) if has_real_merged => Some(assemble_merged(&planes, width, height, colour, transparent, bps)),
        _ => None,
    };

    let layers = set.layers.into_iter().map(|l| l.layer).collect();
    let doc = PsdDoc {
        size: [width, height],
        depth: (bps * 8) as u8,
        psb,
        mode,
        has_real_merged,
        merged,
        layers,
        too_large,
        icc_description,
    };
    Ok(Parsed { doc, flag_offsets, end: c.file_offset() })
}

/// Image resources: returns resource 1057's `hasRealMergedData` (None when absent) and the ICC description.
fn read_resources(c: &mut Cur<'_>) -> Result<(Option<bool>, Option<String>), PsdError> {
    let mut real = None;
    let mut icc = None;
    while c.remaining() > 0 {
        if c.data[c.pos..].iter().all(|&b| b == 0) {
            break;
        }
        c.take(4)?;
        let id = c.u16()?;
        let name_len = c.u8()? as usize;
        // The Pascal name is padded so that length byte + name is even.
        c.take(name_len + (name_len + 1) % 2)?;
        let size = c.u32()? as usize;
        let data = c.take(size)?;
        if !size.is_multiple_of(2) && c.remaining() > 0 {
            c.take(1)?;
        }
        match id {
            1057 if data.len() >= 5 => real = Some(data[4] != 0),
            1039 => icc = icc_description(data),
            _ => {}
        }
    }
    Ok((real, icc))
}

/// The `desc` text of an ICC profile (v2 `desc` or v4 `mluc`), when it can be found.
fn icc_description(p: &[u8]) -> Option<String> {
    let count = be32(p, 128)? as usize;
    for i in 0..count.min(256) {
        let at = 132 + i * 12;
        if p.get(at..at + 4)? != b"desc" {
            continue;
        }
        let off = be32(p, at + 4)? as usize;
        let size = be32(p, at + 8)? as usize;
        let tag = p.get(off..off.checked_add(size)?)?;
        return match tag.get(0..4)? {
            b"desc" => {
                let n = be32(tag, 8)? as usize;
                let ascii = tag.get(12..12usize.checked_add(n)?)?;
                let end = ascii.iter().position(|&b| b == 0).unwrap_or(ascii.len());
                Some(String::from_utf8_lossy(&ascii[..end]).into_owned())
            }
            b"mluc" => {
                let len = be32(tag, 20)? as usize;
                let at = be32(tag, 24)? as usize;
                let units = tag.get(at..at.checked_add(len)?)?;
                Some(utf16be(units))
            }
            _ => None,
        };
    }
    None
}

fn utf16be(b: &[u8]) -> String {
    let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned()
}

/// Windows-1252 code points of the bytes 0x80..=0x9F (undefined ones map to the C1 control of the same number).
const CP1252_HIGH: [u16; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D,
    0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A,
    0x0153, 0x009D, 0x017E, 0x0178,
];

/// A Pascal name: UTF-8 when the bytes are valid UTF-8, else Windows-1252 (13.2 "Names").
fn pascal_name(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_owned(),
        Err(_) => b
            .iter()
            .map(|&x| match x {
                0x80..=0x9F => char::from_u32(CP1252_HIGH[(x - 0x80) as usize] as u32).unwrap_or('?'),
                _ => x as char,
            })
            .collect(),
    }
}

/// One tagged block: its key, its data and the file offset of the data.
type Block<'a> = ([u8; 4], &'a [u8], usize);

/// The tagged blocks of a region, with each block's data and its file offset. Padding between blocks (0 to 3
/// bytes) is found by looking for the next `8BIM` / `8B64`; when that guess fails the walk re-synchronises on the
/// next signature in the region, and ends when there is none.
fn tagged_blocks<'a>(c: &mut Cur<'a>) -> Result<Vec<Block<'a>>, PsdError> {
    let mut out = Vec::new();
    loop {
        if !seek_signature(c) {
            break;
        }
        c.take(4)?;
        let key = c.key4()?;
        let len = if c.psb && psb_long_key(&key) { c.u64()? } else { c.u32()? as u64 };
        if len > c.remaining() as u64 {
            return Err(PsdError::Corrupt("tagged block length"));
        }
        let base = c.file_offset();
        let data = c.take(len as usize)?;
        out.push((key, data, base));
    }
    Ok(out)
}

/// Moves to the next block signature: here, after 1 to 3 padding bytes, or (re-sync) anywhere later in the region.
/// False when the region holds no further signature (trailing padding).
fn seek_signature(c: &mut Cur<'_>) -> bool {
    let rest = &c.data[c.pos..];
    if rest.len() < 12 {
        if rest.iter().any(|&b| b != 0) {
            note_resync();
        }
        return false;
    }
    for skip in 0..4 {
        if rest.len() >= skip + 12 && is_signature(&rest[skip..skip + 4]) {
            if rest[..skip].iter().any(|&b| b != 0) {
                note_resync();
            }
            c.pos += skip;
            return true;
        }
    }
    note_resync();
    match rest.windows(4).position(is_signature) {
        Some(i) if rest.len() - i >= 12 => {
            c.pos += i;
            true
        }
        _ => false,
    }
}

/// The layers of one layer info (the section's, or an `Lr16` / `Lr32` / `Layr` block's).
#[derive(Default)]
struct LayerSet {
    layers: Vec<ParsedLayer>,
    /// The layer count was negative: the merged picture's first extra channel is its transparency.
    negative: bool,
    /// Some layer or mask was over the decode limits.
    too_large: bool,
}

struct ParsedLayer {
    layer: PsdLayer,
    flags_offset: usize,
}

/// What a layer record says before its channel data is read.
struct Record {
    rect: IRect,
    channels: Vec<(i16, u64)>,
    blend: [u8; 4],
    opacity: u8,
    clipping: u8,
    flags: u8,
    flags_offset: usize,
    /// User mask: rect, default colour, flags.
    mask: Option<(IRect, u8, u8)>,
    name: String,
    lyid: Option<u32>,
    features: PsdFeatures,
    section: PsdSection,
    fill_opacity: u8,
    vector_mask: bool,
}

/// `(top, left, bottom, right)` -> rectangle; a rectangle with `bottom < top` or `right < left` (or zero size) is
/// empty.
fn rect_of(top: i32, left: i32, bottom: i32, right: i32) -> IRect {
    if bottom <= top || right <= left { IRect::EMPTY } else { IRect::new(left, top, right, bottom) }
}

fn read_record(c: &mut Cur<'_>) -> Result<Record, PsdError> {
    let top = c.i32()?;
    let left = c.i32()?;
    let bottom = c.i32()?;
    let right = c.i32()?;
    let rect = rect_of(top, left, bottom, right);
    let n = c.u16()?;
    if n > MAX_CHANNELS {
        return Err(PsdError::Corrupt("layer channel count"));
    }
    let mut channels = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let id = c.i16()?;
        let len = c.len()?;
        channels.push((id, len));
    }
    if !is_signature(c.take(4)?) {
        return Err(PsdError::Corrupt("layer record signature"));
    }
    let blend = c.key4()?;
    let opacity = c.u8()?;
    let clipping = c.u8()?;
    let flags_offset = c.file_offset();
    let flags = c.u8()?;
    c.u8()?;
    let n = c.u32()?;
    let mut extra = c.sub(n as u64, "layer record")?;

    // Layer mask data.
    let n = extra.u32()?;
    let mut m = extra.sub(n as u64, "layer mask data")?;
    let mut mask = None;
    let mut features = PsdFeatures::empty();
    if n >= 18 {
        let (t, l, b, r) = (m.i32()?, m.i32()?, m.i32()?, m.i32()?);
        let default = m.u8()?;
        let mflags = m.u8()?;
        if mflags & 0x10 != 0 {
            features |= PsdFeatures::MASK_EXTRA;
        }
        mask = Some((rect_of(t, l, b, r), default, mflags));
    }

    // Blending ranges: default when empty or when every 8-byte pair is the default pair, whatever their number.
    let n = extra.u32()?;
    let ranges = extra.sub(n as u64, "blending ranges")?;
    let blend_if =
        !n.is_multiple_of(8) || ranges.data.as_chunks::<8>().0.iter().any(|p| *p != [0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF]);
    if blend_if {
        features |= PsdFeatures::BLEND_IF;
    }

    // Pascal name, padded so that length byte + name is a multiple of 4 (some writers pad to 2): the padding is
    // left to the block walk, which looks for the first signature within the next 3 bytes.
    let n = extra.u8()? as usize;
    let mut name = pascal_name(extra.take(n)?);

    let mut lyid = None;
    let mut section = PsdSection::Layer;
    let mut blend_key = blend;
    let mut fill_opacity = 255;
    let mut effects_descriptor: Option<bool> = None;
    let mut legacy_effects = false;
    let mut vector_mask = false;
    for (key, data, _) in tagged_blocks(&mut extra)? {
        match &key {
            b"luni" => {
                if let Some(count) = be32(data, 0) {
                    let units = data.get(4..).unwrap_or(&[]);
                    let len = (count as usize).saturating_mul(2).min(units.len() & !1);
                    name = utf16be(&units[..len]);
                }
            }
            b"lyid" => lyid = be32(data, 0),
            b"lsct" | b"lsdk" => {
                if let Some(t) = be32(data, 0) {
                    section = match t {
                        1 => PsdSection::GroupOpen,
                        2 => PsdSection::GroupClosed,
                        3 => PsdSection::GroupEnd,
                        _ => PsdSection::Layer,
                    };
                    if section.is_group_marker() {
                        features |= PsdFeatures::GROUP;
                    }
                    if data.len() >= 12 && is_signature(&data[4..8]) {
                        blend_key = [data[8], data[9], data[10], data[11]];
                    }
                }
            }
            b"artb" => features |= PsdFeatures::GROUP,
            b"lfx2" | b"lmfx" => {
                effects_descriptor = Some(effects_descriptor.unwrap_or(false) || descriptor::effects_enabled(data))
            }
            b"lrFX" => legacy_effects = true,
            b"TySh" | b"tySh" => features |= PsdFeatures::TEXT,
            b"SoLd" | b"PlLd" | b"SoLE" | b"plLd" => features |= PsdFeatures::SMART,
            b"vmsk" | b"vsms" => vector_mask = true,
            b"SoCo" | b"GdFl" | b"PtFl" | b"vscg" | b"vstk" => features |= PsdFeatures::FILL,
            b"iOpa" => fill_opacity = data.first().copied().unwrap_or(255),
            k if ADJUSTMENT_KEYS.contains(&k) => features |= PsdFeatures::ADJUSTMENT,
            _ => {}
        }
    }
    // `lfx2` / `lmfx` decide alone when present; `lrFX` alone (old writers) counts as on.
    if effects_descriptor.unwrap_or(legacy_effects) {
        features |= PsdFeatures::EFFECTS;
    }
    if fill_opacity < 255 {
        features |= PsdFeatures::FILL_OPACITY;
    }
    if let PsdBlend::Other(_) = PsdBlend::from_key(blend_key) {
        features |= PsdFeatures::BLEND_OTHER;
    }
    if channels.iter().any(|&(id, _)| id == -3) {
        features |= PsdFeatures::MASK_EXTRA;
    }
    Ok(Record {
        rect,
        channels,
        blend: blend_key,
        opacity,
        clipping,
        flags,
        flags_offset,
        mask,
        name,
        lyid,
        features,
        section,
        fill_opacity,
        vector_mask,
    })
}

fn read_layer_info(c: &mut Cur<'_>, ctx: &Ctx) -> Result<LayerSet, PsdError> {
    let count = c.i16()?;
    let n = count.unsigned_abs() as usize;
    // A layer record has at least 34 bytes plus its extra data length fields; a count the bytes cannot hold is damage.
    if n.saturating_mul(46) > c.remaining() {
        return Err(c.short.clone());
    }
    let mut records = Vec::with_capacity(n);
    for _ in 0..n {
        records.push(read_record(c)?);
    }
    let mut set = LayerSet { layers: Vec::with_capacity(n), negative: count < 0, too_large: false };
    for rec in records {
        let layer = read_channels(c, rec, ctx, &mut set.too_large)?;
        set.layers.push(layer);
    }
    Ok(set)
}

/// Reads one record's channel data and builds the layer.
fn read_channels(c: &mut Cur<'_>, rec: Record, ctx: &Ctx, too_large: &mut bool) -> Result<ParsedLayer, PsdError> {
    let colour = ctx.colour_channels();
    let marker = rec.section.is_group_marker();
    let layer_fits = rec.rect.area() <= ctx.max_decode_pixels;
    let mask_rect = rec.mask.map(|(r, _, _)| r).unwrap_or(IRect::EMPTY);
    let mask_fits = mask_rect.area() <= ctx.max_decode_pixels;
    if ctx.decode && (!layer_fits || !mask_fits) {
        *too_large = true;
    }
    let decode_layer = ctx.decode && layer_fits && mask_fits && !marker;

    let mut planes: [Option<Vec<u8>>; 4] = [None, None, None, None]; // colour 0..2, then alpha
    let mut mask_plane: Option<Vec<u8>> = None;
    for &(id, len) in &rec.channels {
        let data = c.take_u64(len)?;
        let (rect, slot) = match id {
            -1 => (rec.rect, Some(3)),
            -2 => (mask_rect, None),
            id if id >= 0 && (id as usize) < colour => (rec.rect, Some(id as usize)),
            _ => continue, // -3 (the second mask) and other ids: stepped over
        };
        if data.is_empty() && rect.is_empty() {
            continue;
        }
        if data.len() < 2 {
            return Err(PsdError::Corrupt("channel data length"));
        }
        let compression = u16::from_be_bytes([data[0], data[1]]);
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        if !decode_layer {
            check_compression(compression)?;
            continue;
        }
        let rows = decode_rows(&data[2..], compression, w, h, ctx.bps, ctx.psb)?;
        let plane = to_8bit(rows, ctx.bps);
        match slot {
            Some(i) => planes[i] = Some(plane),
            None if id == -2 => mask_plane = Some(plane),
            None => {}
        }
    }

    let mut features = rec.features;
    let empty = rec.rect.is_empty();
    let has_colour = rec.channels.iter().any(|&(id, _)| id >= 0 && (id as usize) < colour);
    let fill = features.contains(PsdFeatures::FILL);
    // 13.2 "Empty layers and NO_PIXELS": only an empty rect together with flags bits 3 + 4 or a fill / adjustment key,
    // never on a group marker. A layer that has a rect but no colour channel has no picture either.
    let no_pixels = !marker
        && ((empty && (rec.flags & 0x18 == 0x18 || fill || features.contains(PsdFeatures::ADJUSTMENT)))
            || (!empty && !has_colour));
    if no_pixels {
        features |= PsdFeatures::NO_PIXELS;
    }
    // 13.2 "Shape and fill layers": flags bit 4 + a fill key = a shape or fill layer. With a rect it is a finished
    // picture: its vector mask and a rendered (bit 3) user mask are already in its pixels. Without one it is
    // NO_PIXELS (above). VECTOR_MASK is for ordinary pixel layers only.
    let shape_layer = rec.flags & 0x10 != 0 && fill;
    let shape_picture = shape_layer && !empty;
    if rec.vector_mask && !shape_layer {
        features |= PsdFeatures::VECTOR_MASK;
    }

    let pixels = if marker || no_pixels || !decode_layer {
        None
    } else if empty {
        Some(CroppedBuf::empty())
    } else {
        Some(assemble_layer(rec.rect, &planes, colour))
    };
    let mask = match (rec.mask, mask_plane) {
        (Some((rect, default, mflags)), Some(plane)) if !(shape_picture && mflags & 0x08 != 0) => {
            let pixels = AlphaBuf::from_raw(rect.width(), rect.height(), plane)
                .ok_or(PsdError::Corrupt("mask size"))?;
            Some(PsdMask { rect, default, pixels, disabled: mflags & 0x02 != 0, rendered: mflags & 0x08 != 0 })
        }
        (Some((rect, default, mflags)), None)
            if decode_layer && rect.is_empty() && rec.channels.iter().any(|&(id, _)| id == -2) =>
        {
            Some(PsdMask {
                rect: IRect::EMPTY,
                default,
                pixels: AlphaBuf::new(0, 0),
                disabled: mflags & 0x02 != 0,
                rendered: mflags & 0x08 != 0,
            })
        }
        _ => None,
    };

    let layer = PsdLayer {
        name: rec.name,
        lyid: rec.lyid,
        rect: rec.rect,
        pixels,
        mask,
        clipped: rec.clipping != 0,
        visible: rec.flags & 0x02 == 0,
        opacity: rec.opacity,
        blend: PsdBlend::from_key(rec.blend),
        features,
        section: rec.section,
        flags: rec.flags,
        fill_opacity: rec.fill_opacity,
    };
    Ok(ParsedLayer { layer, flags_offset: rec.flags_offset })
}

fn check_compression(compression: u16) -> Result<(), PsdError> {
    if compression > 3 { Err(PsdError::Unsupported(PsdRefusal::Compression(compression))) } else { Ok(()) }
}

/// Decodes `h` rows of `w` samples of `bps` bytes (big-endian) from channel data after its compression word.
fn decode_rows(data: &[u8], compression: u16, w: usize, h: usize, bps: usize, psb: bool) -> Result<Vec<u8>, PsdError> {
    let row = w * bps;
    let total = row.checked_mul(h).ok_or(PsdError::Corrupt("channel size"))?;
    match compression {
        0 => {
            let raw = data.get(..total).ok_or(PsdError::Corrupt("raw channel data too short"))?;
            Ok(raw.to_vec())
        }
        1 => {
            if total == 0 {
                return Ok(Vec::new());
            }
            let (rows, _) = rle_rows(data, row, h, psb)?;
            let mut out = Vec::with_capacity(total);
            for src in rows {
                let decoded = decode_row(src, row).ok_or(PsdError::Corrupt("PackBits row"))?;
                out.extend_from_slice(&decoded);
            }
            Ok(out)
        }
        2 | 3 => {
            if total == 0 {
                return Ok(Vec::new());
            }
            let mut out = inflate(data, total)?;
            if compression == 3 {
                unpredict(&mut out, row, bps);
            }
            Ok(out)
        }
        c => Err(PsdError::Unsupported(PsdRefusal::Compression(c))),
    }
}

/// The PackBits rows of a channel: a table of `h` byte counts (2 bytes each, 4 in a PSB), then the rows. Every row
/// must have at least the bytes that `row` output bytes need (two per started 128), so the output is bounded by the
/// input before anything is allocated.
fn rle_rows(data: &[u8], row: usize, h: usize, psb: bool) -> Result<(Vec<&[u8]>, usize), PsdError> {
    let count_size = if psb { 4 } else { 2 };
    let table = h.checked_mul(count_size).ok_or(PsdError::Corrupt("row table"))?;
    let counts = data.get(..table).ok_or(PsdError::Corrupt("row table"))?;
    let min_row = row.div_ceil(128) * 2;
    let mut at = table;
    let mut rows = Vec::with_capacity(h);
    for c in counts.chunks_exact(count_size) {
        let n = if psb { u32::from_be_bytes([c[0], c[1], c[2], c[3]]) as usize } else { u16::from_be_bytes([c[0], c[1]]) as usize };
        if n < min_row {
            return Err(PsdError::Corrupt("PackBits row too short"));
        }
        let end = at.checked_add(n).ok_or(PsdError::Corrupt("row table"))?;
        rows.push(data.get(at..end).ok_or(PsdError::Corrupt("PackBits rows longer than the channel"))?);
        at = end;
    }
    Ok((rows, at))
}

fn inflate(data: &[u8], total: usize) -> Result<Vec<u8>, PsdError> {
    if total > data.len().saturating_mul(MAX_INFLATE_RATIO).saturating_add(1024) {
        return Err(PsdError::Corrupt("ZIP data too short for its channel"));
    }
    let out = decompress_to_vec_zlib_with_limit(data, total).map_err(|_| PsdError::Corrupt("ZIP data"))?;
    if out.len() != total {
        return Err(PsdError::Corrupt("ZIP data size"));
    }
    Ok(out)
}

/// Undoes ZIP prediction: per row, each sample adds the one before it (8-bit bytes or 16-bit big-endian words,
/// wrapping).
fn unpredict(buf: &mut [u8], row: usize, bps: usize) {
    if row == 0 {
        return;
    }
    for r in buf.chunks_exact_mut(row) {
        if bps == 1 {
            for i in 1..r.len() {
                r[i] = r[i].wrapping_add(r[i - 1]);
            }
        } else {
            let mut prev = 0u16;
            for (i, w) in r.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let v = u16::from_be_bytes([w[0], w[1]]);
                let v = if i == 0 { v } else { v.wrapping_add(prev) };
                w.copy_from_slice(&v.to_be_bytes());
                prev = v;
            }
        }
    }
}

/// 16-bit samples -> 8-bit with `(v + 128) / 257`; 8-bit samples unchanged.
fn to_8bit(rows: Vec<u8>, bps: usize) -> Vec<u8> {
    if bps == 1 {
        return rows;
    }
    rows.as_chunks::<2>().0.iter().map(|w| reduce16(u16::from_be_bytes([w[0], w[1]]))).collect()
}

fn reduce16(v: u16) -> u8 {
    ((v as u32 + 128) / 257) as u8
}

/// A layer's straight RGBA8 pixels from its planes (grey is copied to R, G and B; no alpha channel = opaque; a
/// missing colour channel reads as 0).
fn assemble_layer(rect: IRect, planes: &[Option<Vec<u8>>; 4], colour: usize) -> CroppedBuf {
    let mut buf = CroppedBuf::new(rect);
    let n = rect.area() as usize;
    let get = |p: &Option<Vec<u8>>, i: usize, default: u8| p.as_ref().and_then(|v| v.get(i).copied()).unwrap_or(default);
    for i in 0..n {
        let px = &mut buf.data[i * 4..i * 4 + 4];
        if colour == 1 {
            let g = get(&planes[0], i, 0);
            px[0] = g;
            px[1] = g;
            px[2] = g;
        } else {
            px[0] = get(&planes[0], i, 0);
            px[1] = get(&planes[1], i, 0);
            px[2] = get(&planes[2], i, 0);
        }
        px[3] = get(&planes[3], i, 255);
    }
    buf
}

/// The merged picture's first `need` planes as raw samples (`bps` bytes each), or `None` when pixels are not decoded.
/// The whole section is validated either way (row table and lengths), so a truncated file is an error.
fn read_merged(
    c: &mut Cur<'_>,
    compression: u16,
    width: u32,
    height: u32,
    channels: usize,
    need: usize,
    ctx: &Ctx,
) -> Result<Option<Vec<Vec<u8>>>, PsdError> {
    let (w, h) = (width as usize, height as usize);
    let row = w * ctx.bps;
    let plane = row * h;
    let data = &c.data[c.pos..];
    match compression {
        0 => {
            let total = plane.checked_mul(channels).ok_or(PsdError::Corrupt("merged size"))?;
            if data.len() < total {
                return Err(PsdError::Truncated);
            }
            c.pos += total;
            if !ctx.decode {
                return Ok(None);
            }
            Ok(Some((0..need).map(|i| data[i * plane..(i + 1) * plane].to_vec()).collect()))
        }
        1 => {
            // Running out here is a half-written file.
            let (rows, end) = rle_rows(data, row, h * channels, ctx.psb).map_err(|_| PsdError::Truncated)?;
            c.pos += end;
            if !ctx.decode {
                return Ok(None);
            }
            let mut planes = Vec::with_capacity(need);
            for ch in 0..need {
                let mut out = Vec::with_capacity(plane);
                for src in &rows[ch * h..(ch + 1) * h] {
                    out.extend_from_slice(&decode_row(src, row).ok_or(PsdError::Corrupt("merged PackBits row"))?);
                }
                planes.push(out);
            }
            Ok(Some(planes))
        }
        2 | 3 => {
            if !ctx.decode {
                return Ok(None);
            }
            // One zlib stream for all channels; prediction runs per row of each plane.
            let total = plane.checked_mul(channels).ok_or(PsdError::Corrupt("merged size"))?;
            let mut all = inflate(data, total)?;
            c.pos = c.data.len();
            if compression == 3 {
                unpredict(&mut all, row, ctx.bps);
            }
            Ok(Some((0..need).map(|i| all[i * plane..(i + 1) * plane].to_vec()).collect()))
        }
        c => Err(PsdError::Unsupported(PsdRefusal::Compression(c))),
    }
}

/// Straight RGBA8 from the merged planes: colours un-matted from white when there is a transparency plane
/// (`C = (P - (max - a)) * max / a`, rounded, for `a > 0`; 0 for `a = 0`), at the file's depth, then reduced to 8 bits.
fn assemble_merged(planes: &[Vec<u8>], width: u32, height: u32, colour: usize, transparent: bool, bps: usize) -> PixelBuf {
    let n = width as usize * height as usize;
    let sample = |p: usize, i: usize| -> u32 {
        if bps == 1 { planes[p][i] as u32 } else { u16::from_be_bytes([planes[p][2 * i], planes[p][2 * i + 1]]) as u32 }
    };
    let max: u32 = if bps == 1 { 255 } else { 65535 };
    let to8 = |v: u32| -> u8 { if bps == 1 { v as u8 } else { reduce16(v as u16) } };
    let mut out = PixelBuf::new(width, height);
    for i in 0..n {
        let rgb = if colour == 1 {
            let g = sample(0, i);
            [g, g, g]
        } else {
            [sample(0, i), sample(1, i), sample(2, i)]
        };
        let a = if transparent { sample(colour, i) } else { max };
        let px = &mut out.data[i * 4..i * 4 + 4];
        for k in 0..3 {
            px[k] = if a == max {
                to8(rgb[k])
            } else if a == 0 {
                0
            } else {
                let p = rgb[k] as u64;
                let num = p.saturating_sub((max - a) as u64) * max as u64 + a as u64 / 2;
                to8((num / a as u64).min(max as u64) as u32)
            };
        }
        px[3] = to8(a);
    }
    out
}
