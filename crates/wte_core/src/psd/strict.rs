//! Test-only strict structure walker of PSD / PSB files (STICKER_PLAN 13.3, P5b).
//!
//! It stands in for "Photoshop opens the file cleanly" until a person tries it (hand test 19): it walks a file front
//! to back and fails on any overlap and on any slack. Every length must be exactly what its contents use: section
//! lengths, image resources (Pascal name and data each padded to even with zero bytes), layer records (mask data,
//! blending ranges in whole pairs, the Pascal name padded so that length byte + name is a multiple of 4, layer
//! blocks whose lengths are multiples of 4), every channel's data against its rectangle (raw size, PackBits row
//! counts that add up and rows that decode to exactly one row, ZIP streams that inflate to exactly the channel and end
//! where the channel ends), global blocks (each followed by the zero bytes that pad its length to a multiple of 4),
//! and the merged picture, which must end at the last byte of the file.
//!
//! One allowance, the rule Photoshop's own files follow (13.3, rev 3 review): 0 to 3 **zero** bytes after the channel
//! data at the end of the layer info, and again at the end of the layer-and-mask section, are padding. A non-zero byte
//! there, or more than 3 bytes, fails.
//!
//! It is calibrated on the Photoshop-saved fixtures (`k5_strict_accepts_every_photoshop_fixture`) and then run on every
//! file `write_sticker` produces in the tests.

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};

use crate::export::packbits::decode_row;
use crate::math::IRect;

use super::read::psb_long_key;

/// What the walk found, with the offsets the fault tests patch.
#[derive(Clone, Debug, Default)]
pub struct Walk {
    pub psb: bool,
    pub channels: u16,
    pub size: [u32; 2],
    pub depth: u16,
    pub mode: u16,
    /// Image resource ids in file order, with the offset of each resource's data-length field.
    pub resources: Vec<(u16, usize)>,
    /// Offset of the image resources' section length.
    pub resources_len_at: usize,
    /// Offset of the layer-and-mask section length, and of the layer info length.
    pub section_len_at: usize,
    pub layer_info_len_at: usize,
    /// Layer records of the main layer info (or of the global `Lr16` / `Lr32` / `Layr` block).
    pub layers: usize,
    /// The layer count was negative.
    pub negative: bool,
    /// Per record of the main layer info: the offset of each channel's length field.
    pub channel_len_at: Vec<Vec<usize>>,
    /// Per record of the main layer info: the offset of the extra-data length field.
    pub extra_len_at: Vec<usize>,
    /// Where the main layer info's channel data ended, and the pad bytes after it (0 to 3).
    pub layer_info_data_end: usize,
    pub layer_info_pad: usize,
    /// Pad bytes at the end of the layer-and-mask section (0 to 3).
    pub section_pad: usize,
    /// Global tagged-block keys in file order.
    pub global_blocks: Vec<[u8; 4]>,
    pub merged_compression: u16,
}

type R<T> = Result<T, String>;

struct Cur<'a> {
    data: &'a [u8],
    /// File offset of `data[0]`.
    base: usize,
    pos: usize,
    psb: bool,
    what: &'static str,
}

impl<'a> Cur<'a> {
    fn at(&self) -> usize {
        self.base + self.pos
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn take(&mut self, n: u64) -> R<&'a [u8]> {
        if n > self.remaining() as u64 {
            return Err(format!("{}: {n} bytes wanted at offset {}, {} left", self.what, self.at(), self.remaining()));
        }
        let s = &self.data[self.pos..self.pos + n as usize];
        self.pos += n as usize;
        Ok(s)
    }

    fn sub(&mut self, n: u64, what: &'static str) -> R<Cur<'a>> {
        let base = self.at();
        let data = self.take(n)?;
        Ok(Cur { data, base, pos: 0, psb: self.psb, what })
    }

    fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> R<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn i16(&mut self) -> R<i16> {
        Ok(self.u16()? as i16)
    }

    fn u32(&mut self) -> R<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> R<i32> {
        Ok(self.u32()? as i32)
    }

    fn u64(&mut self) -> R<u64> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    /// 4 bytes in a PSD, 8 in a PSB.
    fn len(&mut self) -> R<u64> {
        if self.psb { self.u64() } else { Ok(self.u32()? as u64) }
    }

    fn key(&mut self) -> R<[u8; 4]> {
        let b = self.take(4)?;
        Ok([b[0], b[1], b[2], b[3]])
    }

    fn zeros(&mut self, n: usize, what: &str) -> R<()> {
        let at = self.at();
        if self.take(n as u64)?.iter().all(|&b| b == 0) {
            Ok(())
        } else {
            Err(format!("{}: non-zero {what} at offset {at}", self.what))
        }
    }

    fn end(&self) -> R<()> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(format!("{}: {} bytes of slack at offset {}", self.what, self.remaining(), self.at()))
        }
    }

    /// The 0-to-3 zero-byte allowance: what remains must be at most 3 bytes, all zero. Returns their number.
    fn pad_allowance(&mut self) -> R<usize> {
        let n = self.remaining();
        if n > 3 {
            return Err(format!("{}: {n} bytes after the data at offset {} (at most 3 pad bytes)", self.what, self.at()));
        }
        self.zeros(n, "pad byte")?;
        Ok(n)
    }
}

/// A signature of a tagged block or layer record.
fn block_signature(b: [u8; 4]) -> bool {
    &b == b"8BIM" || &b == b"8B64"
}

/// Walks a whole file. `Err` names the first part that is not exactly as long as its contents.
pub fn walk(bytes: &[u8]) -> Result<Walk, String> {
    let mut c = Cur { data: bytes, base: 0, pos: 0, psb: false, what: "file" };
    let mut walk = Walk::default();

    // Header.
    if c.take(4)? != b"8BPS" {
        return Err("signature".into());
    }
    let version = c.u16()?;
    if version != 1 && version != 2 {
        return Err(format!("version {version}"));
    }
    walk.psb = version == 2;
    c.psb = walk.psb;
    c.zeros(6, "reserved header byte")?;
    walk.channels = c.u16()?;
    let height = c.u32()?;
    let width = c.u32()?;
    walk.size = [width, height];
    walk.depth = c.u16()?;
    walk.mode = c.u16()?;
    let max_side = if walk.psb { 300_000 } else { 30_000 };
    if !(1..=56).contains(&walk.channels) {
        return Err(format!("channel count {}", walk.channels));
    }
    if width == 0 || height == 0 || width > max_side || height > max_side {
        return Err(format!("canvas {width} x {height}"));
    }
    if ![1, 8, 16, 32].contains(&walk.depth) {
        return Err(format!("depth {}", walk.depth));
    }
    if ![0, 1, 2, 3, 4, 7, 8, 9].contains(&walk.mode) {
        return Err(format!("colour mode {}", walk.mode));
    }

    // Colour mode data.
    let n = c.u32()?;
    c.take(n as u64)?;

    // Image resources.
    walk.resources_len_at = c.at();
    let n = c.u32()?;
    let mut res = c.sub(n as u64, "image resources")?;
    while res.remaining() > 0 {
        let sig = res.key()?;
        if !matches!(&sig, b"8BIM" | b"MeSa" | b"PHUT" | b"AgHg" | b"DCSR") {
            return Err(format!("image resource signature {:?} at offset {}", String::from_utf8_lossy(&sig), res.at() - 4));
        }
        let id = res.u16()?;
        let name_len = res.u8()? as usize;
        res.take(name_len as u64)?;
        if (1 + name_len) % 2 == 1 {
            res.zeros(1, "resource name pad")?;
        }
        let len_at = res.at();
        let len = res.u32()? as usize;
        res.take(len as u64).map_err(|e| format!("resource {id}: {e}"))?;
        if len % 2 == 1 {
            if res.remaining() == 0 {
                return Err(format!("resource {id}: data of odd length {len} is not padded to even"));
            }
            res.zeros(1, "resource data pad")?;
        }
        walk.resources.push((id, len_at));
    }

    // Layer and mask information.
    walk.section_len_at = c.at();
    let n = c.len()?;
    let mut lm = c.sub(n, "layer and mask information")?;
    if n > 0 {
        if n % 4 != 0 {
            return Err(format!("layer and mask section length {n} is not a multiple of 4"));
        }
        walk.layer_info_len_at = lm.at();
        let n = lm.len()?;
        if n % 4 != 0 {
            return Err(format!("layer info length {n} is not a multiple of 4"));
        }
        let mut li = lm.sub(n, "layer info")?;
        if n > 0 {
            layer_info(&mut li, &mut walk, true)?;
        }
        let n = lm.u32()?;
        lm.take(n as u64).map_err(|e| format!("global layer mask info: {e}"))?;
        while lm.remaining() > 3 {
            let block_at = lm.at();
            let sig = lm.key()?;
            if !block_signature(sig) {
                return Err(format!("global block signature at offset {block_at}"));
            }
            let key = lm.key()?;
            let n = if walk.psb && psb_long_key(&key) { lm.u64()? } else { lm.u32()? as u64 };
            let mut data = lm.sub(n, "global block")?;
            if matches!(&key, b"Lr16" | b"Lr32" | b"Layr") && n > 0 {
                let mut sub = Walk { depth: walk.depth, ..Walk::default() };
                layer_info(&mut data, &mut sub, false)?;
                if walk.layers == 0 {
                    walk.layers = sub.layers;
                    walk.negative = sub.negative;
                }
            }
            let pad = (n.div_ceil(4) * 4 - n) as usize;
            lm.zeros(pad, "global block pad").map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&key)))?;
            walk.global_blocks.push(key);
        }
        walk.section_pad = lm.pad_allowance()?;
    }

    // Merged picture: the rest of the file.
    walk.merged_compression = c.u16()?;
    let bps = (walk.depth as usize).div_ceil(8);
    let row = if walk.depth == 1 { (width as usize).div_ceil(8) } else { width as usize * bps };
    let rows = walk.channels as usize * height as usize;
    let rest = c.remaining();
    let mut merged = c.sub(rest as u64, "merged picture")?;
    channel_body(&mut merged, walk.merged_compression, row, rows)?;
    Ok(walk)
}

/// One layer info (from its count to its end), in the section or in a global `Lr16` / `Lr32` / `Layr` block.
fn layer_info(c: &mut Cur<'_>, walk: &mut Walk, main: bool) -> R<()> {
    let count = c.i16()?;
    walk.negative = count < 0;
    walk.layers = count.unsigned_abs() as usize;
    let bps = (walk.depth as usize).div_ceil(8).max(1);
    let mut records = Vec::with_capacity(walk.layers);
    for _ in 0..walk.layers {
        let rec = record(c)?;
        if main {
            walk.channel_len_at.push(rec.channel_len_at.clone());
            walk.extra_len_at.push(rec.extra_len_at);
        }
        records.push(rec);
    }
    for (i, rec) in records.iter().enumerate() {
        for &(id, len) in &rec.channels {
            let rect = match id {
                -2 => rec.mask,
                -3 => rec.real_mask,
                _ => rec.rect,
            };
            let (w, h) = (rect.width() as usize, rect.height() as usize);
            let mut ch = c.sub(len, "channel data").map_err(|e| format!("layer {i} channel {id}: {e}"))?;
            let compression = ch.u16().map_err(|e| format!("layer {i} channel {id}: {e}"))?;
            channel_body(&mut ch, compression, w * bps, h).map_err(|e| format!("layer {i} channel {id}: {e}"))?;
        }
    }
    if main {
        walk.layer_info_data_end = c.at();
        walk.layer_info_pad = c.pad_allowance()?;
    } else {
        c.pad_allowance()?;
    }
    Ok(())
}

/// One layer record's facts.
struct Record {
    rect: IRect,
    mask: IRect,
    real_mask: IRect,
    channels: Vec<(i16, u64)>,
    channel_len_at: Vec<usize>,
    extra_len_at: usize,
}

fn rect(c: &mut Cur<'_>) -> R<IRect> {
    let (top, left, bottom, right) = (c.i32()?, c.i32()?, c.i32()?, c.i32()?);
    Ok(if bottom <= top || right <= left { IRect::EMPTY } else { IRect::new(left, top, right, bottom) })
}

fn record(c: &mut Cur<'_>) -> R<Record> {
    let start = c.at();
    let layer_rect = rect(c)?;
    let n = c.u16()?;
    if n > 56 {
        return Err(format!("record at {start}: {n} channels"));
    }
    let mut channels = Vec::with_capacity(n as usize);
    let mut channel_len_at = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let id = c.i16()?;
        if id < -3 {
            return Err(format!("record at {start}: channel id {id}"));
        }
        channel_len_at.push(c.at());
        let len = c.len()?;
        if len < 2 {
            return Err(format!("record at {start}: channel {id} has {len} bytes (the compression word needs 2)"));
        }
        channels.push((id, len));
    }
    if !block_signature(c.key()?) {
        return Err(format!("record at {start}: blend signature"));
    }
    c.key()?;
    c.u8()?;
    let clipping = c.u8()?;
    if clipping > 1 {
        return Err(format!("record at {start}: clipping byte {clipping}"));
    }
    c.u8()?;
    c.zeros(1, "record filler")?;
    let extra_len_at = c.at();
    let n = c.u32()?;
    let mut extra = c.sub(n as u64, "layer extra data")?;

    // Layer mask data.
    let n = extra.u32()?;
    let mut m = extra.sub(n as u64, "layer mask data")?;
    let (mut mask, mut real_mask) = (IRect::EMPTY, IRect::EMPTY);
    if n > 0 {
        if n < 20 {
            return Err(format!("record at {start}: mask data of {n} bytes"));
        }
        mask = rect(&mut m)?;
        m.u8()?;
        let flags = m.u8()?;
        if n == 20 {
            m.zeros(2, "mask data pad")?;
        } else {
            // The longer form: real flags, real background and the real mask's rectangle, then (flags bit 4) the
            // mask parameters: a flags byte and a density byte / feather double per bit.
            m.u8()?;
            m.u8()?;
            real_mask = rect(&mut m)?;
            if flags & 0x10 != 0 {
                let params = m.u8()?;
                for (bit, size) in [(0x01, 1), (0x02, 8), (0x04, 1), (0x08, 8)] {
                    if params & bit != 0 {
                        m.take(size)?;
                    }
                }
            }
            let rest = m.remaining();
            if rest > 3 {
                return Err(format!("record at {start}: {rest} bytes after the mask data"));
            }
            m.zeros(rest, "mask data pad")?;
        }
        m.end()?;
    }

    // Blending ranges: whole 8-byte pairs.
    let n = extra.u32()?;
    if n % 8 != 0 {
        return Err(format!("record at {start}: blending ranges of {n} bytes"));
    }
    extra.take(n as u64)?;

    // Pascal name, padded so that length byte + name is a multiple of 4.
    let name_len = extra.u8()? as usize;
    extra.take(name_len as u64)?;
    let pad = (1 + name_len).div_ceil(4) * 4 - (1 + name_len);
    extra.zeros(pad, "name pad").map_err(|e| format!("record at {start}: {e}"))?;

    // Layer blocks: lengths are multiples of 4 and fill the extra data exactly.
    while extra.remaining() > 0 {
        let block_at = extra.at();
        if !block_signature(extra.key()?) {
            return Err(format!("record at {start}: block signature at offset {block_at}"));
        }
        let key = extra.key()?;
        let n = if c.psb && psb_long_key(&key) { extra.u64()? } else { extra.u32()? as u64 };
        if n % 4 != 0 {
            return Err(format!(
                "record at {start}: block {} length {n} is not a multiple of 4",
                String::from_utf8_lossy(&key)
            ));
        }
        extra.take(n).map_err(|e| format!("block {}: {e}", String::from_utf8_lossy(&key)))?;
    }
    Ok(Record { rect: layer_rect, mask, real_mask, channels, channel_len_at, extra_len_at })
}

/// One channel (or the whole merged picture) after its compression word: `rows` rows of `row` bytes. The data must
/// be used exactly.
fn channel_body(c: &mut Cur<'_>, compression: u16, row: usize, rows: usize) -> R<()> {
    let total = row.checked_mul(rows).ok_or("channel size overflows")?;
    match compression {
        0 => {
            c.take(total as u64)?;
        }
        1 => {
            if total == 0 {
                return c.end();
            }
            let mut counts = Vec::with_capacity(rows);
            for _ in 0..rows {
                counts.push(if c.psb { c.u32()? as u64 } else { c.u16()? as u64 });
            }
            for (y, &n) in counts.iter().enumerate() {
                let src = c.take(n).map_err(|e| format!("PackBits row {y}: {e}"))?;
                if decode_row(src, row).is_none() {
                    return Err(format!("PackBits row {y} at offset {} does not decode to {row} bytes", c.at() - n as usize));
                }
            }
        }
        2 | 3 => {
            let rest = c.remaining();
            let data = c.take(rest as u64)?;
            let mut out = vec![0u8; total + 1];
            let mut state = DecompressorOxide::new();
            let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
            let (status, used, written) = decompress(&mut state, data, &mut out, 0, flags);
            if status != TINFLStatus::Done || used != data.len() || written != total {
                return Err(format!(
                    "ZIP data: status {status:?}, {used} of {} bytes used, {written} of {total} bytes made",
                    data.len()
                ));
            }
        }
        other => return Err(format!("compression {other}")),
    }
    c.end()
}
