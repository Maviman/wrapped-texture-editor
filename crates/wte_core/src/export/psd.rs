// Owner: core-persist (C6). Layered PSD writer of ARCHITECTURE.md 7.7 (8-bit RGB, PSD version 1, PackBits) and the
// strict reader that verifies it in tests.
//
// The writer needs every channel length before the first layer record, but `W` cannot seek. It therefore makes two
// passes over the pixels: the first only measures the PackBits rows, the second encodes them again and writes. Memory
// stays bounded by one channel of one layer, whatever the number of layers.

use std::io::Write;

use rayon::prelude::*;

use crate::composite::opacity_u8;
use crate::error::ExportError;
use crate::export::packbits::{decode_row, encode_row};
use crate::export::{ExportLayer, ExportSet};
use crate::math::IRect;
use crate::pixel::{CroppedBuf, PixelBuf, PixelView};
use crate::pixel_ops::content_bounds;

/// Largest width or height of a version-1 PSD.
const MAX_SIDE: u32 = 30000;
/// Channel ids of a layer in file order: transparency, red, green, blue.
const LAYER_CHANNELS: [i16; 4] = [-1, 0, 1, 2];
/// Byte offset inside an RGBA pixel for each entry of `LAYER_CHANNELS`.
const LAYER_CHANNEL_OFFSETS: [usize; 4] = [3, 0, 1, 2];

fn round_up4(n: usize) -> usize {
    n.div_ceil(4) * 4
}

fn invalid_input(message: &str) -> ExportError {
    ExportError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, message.to_owned()))
}

/// Pascal string padded so that `1 + len` is a multiple of 4; ASCII only (anything else becomes `?`), at most 255 bytes.
fn pascal_name(name: &str) -> Vec<u8> {
    let bytes: Vec<u8> = name.chars().take(255).map(|c| if c.is_ascii() { c as u8 } else { b'?' }).collect();
    let mut out = Vec::with_capacity(round_up4(1 + bytes.len()));
    out.push(bytes.len() as u8);
    out.extend_from_slice(&bytes);
    out.resize(round_up4(out.len()), 0);
    out
}

/// The `luni` additional-information block holding the full layer name as UTF-16BE.
fn unicode_name_block(name: &str) -> Vec<u8> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let block_len = round_up4(4 + 2 * units.len());
    let mut out = Vec::with_capacity(12 + block_len);
    out.extend_from_slice(b"8BIMluni");
    out.extend_from_slice(&(block_len as u32).to_be_bytes());
    out.extend_from_slice(&(units.len() as u32).to_be_bytes());
    for unit in units {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out.resize(12 + block_len, 0);
    out
}

/// The bytes of `view` for canvas row `y` between `x0` and `x1`; the caller guarantees the span lies inside the view.
fn row_span<'a>(view: &PixelView<'a>, y: i32, x0: i32, x1: i32) -> &'a [u8] {
    let row = view.row(y).unwrap_or(&[]);
    let start = (x0 - view.rect.x0) as usize * 4;
    let end = (x1 - view.rect.x0) as usize * 4;
    row.get(start..end).unwrap_or(&[])
}

/// One plane of an RGBA row.
fn extract_plane(rgba: &[u8], offset: usize, plane: &mut Vec<u8>) {
    plane.clear();
    plane.extend(rgba.as_chunks::<4>().0.iter().map(|px| px[offset]));
}

/// One plane of a merged-image row: R, G, B (matted on white when `matte`) or A.
fn merged_plane(rgba: &[u8], channel: usize, matte: bool, plane: &mut Vec<u8>) {
    if channel == 3 || !matte {
        extract_plane(rgba, channel, plane);
        return;
    }
    plane.clear();
    plane.extend(rgba.as_chunks::<4>().0.iter().map(|px| {
        let a = px[3] as u32;
        ((px[channel] as u32 * a + 255 * (255 - a) + 127) / 255) as u8
    }));
}

/// What the writer knows about a layer after the measuring pass.
struct LayerPlan<'a> {
    layer: &'a ExportLayer<'a>,
    /// Content bounding box inside the canvas; `IRect::EMPTY` for an empty layer.
    rect: IRect,
    /// PackBits length of every row, per channel in `LAYER_CHANNELS` order (empty for an empty layer).
    row_lens: Vec<[u16; 4]>,
    pascal: Vec<u8>,
    luni: Vec<u8>,
}

impl LayerPlan<'_> {
    /// Length of one channel's data including the 2-byte compression word.
    fn channel_len(&self, channel: usize) -> u64 {
        if self.rect.is_empty() {
            return 2;
        }
        2 + 2 * self.row_lens.len() as u64 + self.row_lens.iter().map(|l| l[channel] as u64).sum::<u64>()
    }

    fn extra_len(&self) -> u64 {
        (4 + 4 + self.pascal.len() + self.luni.len()) as u64
    }

    fn record_len(&self) -> u64 {
        16 + 2 + 6 * 4 + 4 + 4 + 4 + 4 + self.extra_len()
    }
}

/// PackBits lengths of the rows `rect` of `view`, all four layer channels at once.
fn measure_layer(view: &PixelView<'_>, rect: IRect) -> Vec<[u16; 4]> {
    (rect.y0..rect.y1)
        .into_par_iter()
        .map_init(
            || (Vec::new(), Vec::new()),
            |(plane, encoded), y| {
                let rgba = row_span(view, y, rect.x0, rect.x1);
                let mut lens = [0u16; 4];
                for (len, &offset) in lens.iter_mut().zip(&LAYER_CHANNEL_OFFSETS) {
                    extract_plane(rgba, offset, plane);
                    encoded.clear();
                    encode_row(plane, encoded);
                    *len = encoded.len() as u16;
                }
                lens
            },
        )
        .collect()
}

/// Writes one RLE channel: the row lengths, then the PackBits rows produced by `plane_of(y, plane)`.
fn write_rle_rows<W: Write>(
    w: &mut W,
    rows: std::ops::Range<i32>,
    lens: impl Iterator<Item = u16>,
    plane_of: impl Fn(i32, &mut Vec<u8>) + Sync,
) -> Result<(), ExportError> {
    let mut table = Vec::with_capacity(rows.len() * 2);
    for len in lens {
        table.extend_from_slice(&len.to_be_bytes());
    }
    w.write_all(&table)?;
    let encoded: Vec<Vec<u8>> = rows
        .into_par_iter()
        .map_init(Vec::new, |plane, y| {
            plane_of(y, plane);
            let mut out = Vec::with_capacity(plane.len() / 8 + 8);
            encode_row(plane, &mut out);
            out
        })
        .collect();
    for row in &encoded {
        w.write_all(row)?;
    }
    Ok(())
}

/// Writes one texture set as a layered PSD (section 7.7).
///
/// Layer rectangles are the content bounding boxes (alpha > 0) clipped to the canvas; the pixels are straight alpha.
/// Errors: `PsdDimension` for a side of 0 or above 30000, `PsdTooLarge` when the layer section or the file would
/// exceed `u32::MAX` bytes (nothing is written in either case), `Io` for a `flattened` image of the wrong size and for
/// write failures.
pub fn write_psd<W: std::io::Write>(w: W, set: &ExportSet<'_>) -> Result<(), ExportError> {
    let mut w = w;
    let (width, height) = (set.width, set.height);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(ExportError::PsdDimension { width, height });
    }
    let flat = set.flattened;
    if flat.width != width || flat.height != height || flat.data.len() != width as usize * height as usize * 4 {
        return Err(invalid_input("the flattened image does not have the size of the texture set"));
    }
    if set.layers.len() > i16::MAX as usize {
        return Err(invalid_input("too many layers for a PSD file"));
    }
    let canvas = IRect::from_size(width, height);

    // Pass 1: measure.
    let plans: Vec<LayerPlan<'_>> = set
        .layers
        .iter()
        .map(|layer| {
            let rect = content_bounds(layer.pixels).map_or(IRect::EMPTY, |r| r.intersect(&canvas));
            let rect = if rect.is_empty() { IRect::EMPTY } else { rect };
            let row_lens = if rect.is_empty() { Vec::new() } else { measure_layer(&layer.pixels, rect) };
            LayerPlan { layer, rect, row_lens, pascal: pascal_name(layer.name), luni: unicode_name_block(layer.name) }
        })
        .collect();

    let matte = flat.data.as_chunks::<4>().0.par_iter().any(|px| px[3] != 255);
    let stride = width as usize * 4;
    let merged_lens: Vec<[u16; 4]> = flat
        .data
        .par_chunks(stride)
        .map_init(
            || (Vec::new(), Vec::new()),
            |(plane, encoded), rgba| {
                let mut lens = [0u16; 4];
                for (channel, len) in lens.iter_mut().enumerate() {
                    merged_plane(rgba, channel, matte, plane);
                    encoded.clear();
                    encode_row(plane, encoded);
                    *len = encoded.len() as u16;
                }
                lens
            },
        )
        .collect();

    // Sizes, in u64.
    let records: u64 = plans.iter().map(LayerPlan::record_len).sum();
    let channel_data: u64 = plans.iter().map(|p| (0..4).map(|c| p.channel_len(c)).sum::<u64>()).sum();
    let layer_info_unpadded = 2 + records + channel_data;
    let layer_info_len = layer_info_unpadded.div_ceil(4) * 4;
    let section_len = 4 + layer_info_len + 4;
    let merged_len: u64 =
        2 + 8 * height as u64 + merged_lens.iter().map(|l| l.iter().map(|&v| v as u64).sum::<u64>()).sum::<u64>();
    const RESOURCES_LEN: u64 = 28;
    let file_len = 26 + 4 + (4 + RESOURCES_LEN) + (4 + section_len) + merged_len;
    if section_len > u32::MAX as u64 || file_len > u32::MAX as u64 {
        return Err(ExportError::PsdTooLarge { bytes: file_len });
    }

    // A. Header, B. colour mode data, C. image resources (resolution info: 72 dpi).
    let mut head = Vec::with_capacity(64);
    head.extend_from_slice(b"8BPS");
    head.extend_from_slice(&1u16.to_be_bytes());
    head.extend_from_slice(&[0; 6]);
    head.extend_from_slice(&4u16.to_be_bytes());
    head.extend_from_slice(&height.to_be_bytes());
    head.extend_from_slice(&width.to_be_bytes());
    head.extend_from_slice(&8u16.to_be_bytes());
    head.extend_from_slice(&3u16.to_be_bytes());
    head.extend_from_slice(&0u32.to_be_bytes());
    head.extend_from_slice(&(RESOURCES_LEN as u32).to_be_bytes());
    head.extend_from_slice(b"8BIM");
    head.extend_from_slice(&0x03EDu16.to_be_bytes());
    head.extend_from_slice(&[0, 0]);
    head.extend_from_slice(&16u32.to_be_bytes());
    for _ in 0..2 {
        head.extend_from_slice(&[0x00, 0x48, 0x00, 0x00]);
        head.extend_from_slice(&1u16.to_be_bytes());
        head.extend_from_slice(&1u16.to_be_bytes());
    }

    // D. Layer and mask information: lengths, layer count, records.
    head.extend_from_slice(&(section_len as u32).to_be_bytes());
    head.extend_from_slice(&(layer_info_len as u32).to_be_bytes());
    head.extend_from_slice(&(-(plans.len() as i16)).to_be_bytes());
    for plan in &plans {
        let r = plan.rect;
        for v in [r.y0, r.x0, r.y1, r.x1] {
            head.extend_from_slice(&v.to_be_bytes());
        }
        head.extend_from_slice(&4u16.to_be_bytes());
        for (channel, id) in LAYER_CHANNELS.iter().enumerate() {
            head.extend_from_slice(&id.to_be_bytes());
            head.extend_from_slice(&(plan.channel_len(channel) as u32).to_be_bytes());
        }
        head.extend_from_slice(b"8BIM");
        head.extend_from_slice(&plan.layer.blend.psd_key());
        head.push(opacity_u8(plan.layer.opacity));
        head.push(0);
        head.push(0x08 | if plan.layer.visible { 0 } else { 0x02 });
        head.push(0);
        head.extend_from_slice(&(plan.extra_len() as u32).to_be_bytes());
        head.extend_from_slice(&0u32.to_be_bytes());
        head.extend_from_slice(&0u32.to_be_bytes());
        head.extend_from_slice(&plan.pascal);
        head.extend_from_slice(&plan.luni);
    }
    w.write_all(&head)?;

    // Pass 2: channel image data, layer by layer.
    for plan in &plans {
        if plan.rect.is_empty() {
            for _ in 0..4 {
                w.write_all(&0u16.to_be_bytes())?;
            }
            continue;
        }
        let (view, rect) = (plan.layer.pixels, plan.rect);
        for (channel, &offset) in LAYER_CHANNEL_OFFSETS.iter().enumerate() {
            w.write_all(&1u16.to_be_bytes())?;
            write_rle_rows(&mut w, rect.y0..rect.y1, plan.row_lens.iter().map(|l| l[channel]), |y, plane| {
                extract_plane(row_span(&view, y, rect.x0, rect.x1), offset, plane)
            })?;
        }
    }
    w.write_all(&vec![0u8; (layer_info_len - layer_info_unpadded) as usize])?;
    w.write_all(&0u32.to_be_bytes())?;

    // E. Merged image: all row lengths in plane order R, G, B, A, then the rows.
    w.write_all(&1u16.to_be_bytes())?;
    let mut table = Vec::with_capacity(height as usize * 8);
    for channel in 0..4 {
        for lens in &merged_lens {
            table.extend_from_slice(&lens[channel].to_be_bytes());
        }
    }
    w.write_all(&table)?;
    for channel in 0..4 {
        write_rle_rows(&mut w, 0..height as i32, std::iter::empty(), |y, plane| {
            merged_plane(flat.row(y as u32), channel, matte, plane)
        })?;
    }
    w.flush()?;
    Ok(())
}

/// Cursor over the bytes of a PSD; every read fails instead of running past the end.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    what: &'static str,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8], what: &'static str) -> Cursor<'a> {
        Cursor { data, pos: 0, what }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.remaining() {
            return Err(format!("{}: {n} bytes wanted at offset {}, {} left", self.what, self.pos, self.remaining()));
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn sub(&mut self, n: usize, what: &'static str) -> Result<Cursor<'a>, String> {
        Ok(Cursor::new(self.take(n)?, what))
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn i16(&mut self) -> Result<i16, String> {
        Ok(self.u16()? as i16)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }

    fn tag(&mut self, expected: &[u8]) -> Result<(), String> {
        let at = self.pos;
        let found = self.take(expected.len())?;
        if found == expected {
            Ok(())
        } else {
            Err(format!(
                "{}: expected {:?} at offset {at}, found {:?}",
                self.what,
                String::from_utf8_lossy(expected),
                String::from_utf8_lossy(found)
            ))
        }
    }

    fn zeros(&mut self, n: usize) -> Result<(), String> {
        let at = self.pos;
        if self.take(n)?.iter().all(|&b| b == 0) { Ok(()) } else { Err(format!("{}: non-zero padding at offset {at}", self.what)) }
    }

    fn finish(&self) -> Result<(), String> {
        if self.remaining() == 0 { Ok(()) } else { Err(format!("{}: {} unread bytes", self.what, self.remaining())) }
    }
}

/// Decodes one channel (`width` x `height` bytes) from a cursor holding exactly the channel's data.
fn read_channel(c: &mut Cursor<'_>, width: usize, height: usize) -> Result<Vec<u8>, String> {
    let compression = c.u16()?;
    let plane = read_plane_body(c, compression, width, height)?;
    c.finish()?;
    Ok(plane)
}

/// The rows of one plane after its compression word: raw bytes (0) or a row-length table followed by PackBits rows (1).
fn read_plane_body(c: &mut Cursor<'_>, compression: u16, width: usize, height: usize) -> Result<Vec<u8>, String> {
    match compression {
        0 => Ok(c.take(width.checked_mul(height).ok_or("plane size overflows")?)?.to_vec()),
        1 => {
            let lens: Vec<usize> = (0..height).map(|_| c.u16().map(usize::from)).collect::<Result<_, _>>()?;
            read_rle_rows(c, &lens, width)
        }
        other => Err(format!("{}: unsupported compression {other}", c.what)),
    }
}

fn read_rle_rows(c: &mut Cursor<'_>, lens: &[usize], width: usize) -> Result<Vec<u8>, String> {
    let mut plane = Vec::new();
    for (y, &len) in lens.iter().enumerate() {
        let row = decode_row(c.take(len)?, width).ok_or_else(|| format!("{}: malformed PackBits data in row {y}", c.what))?;
        plane.extend_from_slice(&row);
    }
    Ok(plane)
}

/// A layer record before its pixels are known.
struct RawRecord {
    rect: IRect,
    channel_lens: [usize; 4],
    blend_key: [u8; 4],
    opacity: u8,
    hidden: bool,
    name: String,
    unicode_name: String,
}

fn read_record(c: &mut Cursor<'_>) -> Result<RawRecord, String> {
    let (top, left, bottom, right) = (c.i32()?, c.i32()?, c.i32()?, c.i32()?);
    if bottom < top || right < left {
        return Err(format!("layer rectangle {left},{top} .. {right},{bottom} is inverted"));
    }
    if c.u16()? != 4 {
        return Err("a layer does not have exactly 4 channels".to_owned());
    }
    let mut channel_lens = [0usize; 4];
    for (len, expected_id) in channel_lens.iter_mut().zip(LAYER_CHANNELS) {
        let id = c.i16()?;
        if id != expected_id {
            return Err(format!("channel id {id} where {expected_id} was expected"));
        }
        *len = c.u32()? as usize;
    }
    c.tag(b"8BIM")?;
    let key = c.take(4)?;
    let blend_key = [key[0], key[1], key[2], key[3]];
    let opacity = c.u8()?;
    if c.u8()? != 0 {
        return Err("layer clipping is not 0".to_owned());
    }
    let flags = c.u8()?;
    if flags & !0x02 != 0x08 {
        return Err(format!("unexpected layer flags {flags:#04x}"));
    }
    if c.u8()? != 0 {
        return Err("layer filler byte is not 0".to_owned());
    }
    let extra_len = c.u32()? as usize;
    let mut extra = c.sub(extra_len, "layer extra data")?;
    if extra.u32()? != 0 {
        return Err("layer mask data is not empty".to_owned());
    }
    if extra.u32()? != 0 {
        return Err("layer blending ranges are not empty".to_owned());
    }
    let name_len = extra.u8()? as usize;
    let name_bytes = extra.take(name_len)?;
    if !name_bytes.is_ascii() {
        return Err("the Pascal layer name is not ASCII".to_owned());
    }
    let name = String::from_utf8_lossy(name_bytes).into_owned();
    extra.zeros(round_up4(1 + name_len) - (1 + name_len))?;

    let mut unicode_name = None;
    while extra.remaining() > 0 {
        extra.tag(b"8BIM")?;
        let key = extra.take(4)?;
        let block_len = extra.u32()? as usize;
        if !block_len.is_multiple_of(4) {
            return Err("additional layer information length is not a multiple of 4".to_owned());
        }
        let mut block = extra.sub(block_len, "additional layer information")?;
        if key == b"luni" {
            let units = block.u32()? as usize;
            if block_len != round_up4(4 + 2 * units) {
                return Err("unicode name block has the wrong length".to_owned());
            }
            let utf16: Vec<u16> = (0..units).map(|_| block.u16()).collect::<Result<_, _>>()?;
            let padding = block.remaining();
            block.zeros(padding)?;
            let text = String::from_utf16(&utf16).map_err(|_| "unicode layer name is not valid UTF-16".to_owned())?;
            if unicode_name.replace(text).is_some() {
                return Err("two unicode name blocks in one layer".to_owned());
            }
        }
    }
    let unicode_name = unicode_name.ok_or("layer has no unicode name block")?;
    Ok(RawRecord {
        rect: IRect::new(left, top, right, bottom),
        channel_lens,
        blend_key,
        opacity,
        hidden: flags & 0x02 != 0,
        name,
        unicode_name,
    })
}

/// Strict verification reader for the PSDs written by `write_psd` (tests).
///
/// Fails when any declared length disagrees with the bytes consumed, when padding is not zero or when bytes remain
/// after the merged image. `merged` holds the stored planes (RGB matted on white when the file has transparency).
#[doc(hidden)]
pub fn read_psd(bytes: &[u8]) -> Result<PsdDoc, String> {
    let mut c = Cursor::new(bytes, "file");

    // A. Header.
    c.tag(b"8BPS")?;
    if c.u16()? != 1 {
        return Err("not a version-1 PSD".to_owned());
    }
    c.zeros(6)?;
    if c.u16()? != 4 {
        return Err("the file does not have 4 channels".to_owned());
    }
    let (height, width) = (c.u32()?, c.u32()?);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("invalid canvas size {width}x{height}"));
    }
    if c.u16()? != 8 {
        return Err("depth is not 8 bits".to_owned());
    }
    if c.u16()? != 3 {
        return Err("colour mode is not RGB".to_owned());
    }

    // B. Colour mode data.
    if c.u32()? != 0 {
        return Err("colour mode data is not empty".to_owned());
    }

    // C. Image resources.
    let resources_len = c.u32()? as usize;
    let mut resources = c.sub(resources_len, "image resources")?;
    while resources.remaining() > 0 {
        resources.tag(b"8BIM")?;
        let id = resources.u16()?;
        let name_len = resources.u8()? as usize;
        resources.take(name_len)?;
        if !(1 + name_len).is_multiple_of(2) {
            resources.zeros(1)?;
        }
        let size = resources.u32()? as usize;
        resources.take(size)?;
        if !size.is_multiple_of(2) {
            resources.zeros(1)?;
        }
        if id == 0x03ED && size != 16 {
            return Err("resolution info has the wrong size".to_owned());
        }
    }

    // D. Layer and mask information.
    let section_len = c.u32()? as usize;
    let mut section = c.sub(section_len, "layer and mask information")?;
    let layer_info_len = section.u32()? as usize;
    if !layer_info_len.is_multiple_of(4) {
        return Err(format!("layer info length {layer_info_len} is not a multiple of 4"));
    }
    if section_len != 4 + layer_info_len + 4 {
        return Err(format!("section length {section_len} != 4 + {layer_info_len} + 4"));
    }
    let mut info = section.sub(layer_info_len, "layer info")?;
    let count = info.i16()?.unsigned_abs() as usize;
    let records: Vec<RawRecord> = (0..count).map(|_| read_record(&mut info)).collect::<Result<_, _>>()?;
    let mut layers = Vec::with_capacity(count);
    for record in records {
        let (w, h) = (record.rect.width() as usize, record.rect.height() as usize);
        let mut planes: Vec<Vec<u8>> = Vec::with_capacity(4);
        for len in record.channel_lens {
            let mut channel = info.sub(len, "channel data")?;
            planes.push(read_channel(&mut channel, w, h)?);
        }
        let pixels = if record.rect.is_empty() {
            CroppedBuf::empty()
        } else {
            let mut data = vec![0u8; w * h * 4];
            for (plane, offset) in planes.iter().zip(LAYER_CHANNEL_OFFSETS) {
                for (px, &v) in data.as_chunks_mut::<4>().0.iter_mut().zip(plane) {
                    px[offset] = v;
                }
            }
            CroppedBuf { rect: record.rect, data }
        };
        layers.push(PsdLayer {
            rect: record.rect,
            blend_key: record.blend_key,
            opacity: record.opacity,
            hidden: record.hidden,
            name: record.name,
            unicode_name: record.unicode_name,
            pixels,
        });
    }
    let padding = info.remaining();
    if padding >= 4 {
        return Err(format!("layer info: {padding} unread bytes"));
    }
    info.zeros(padding)?;
    if section.u32()? != 0 {
        return Err("global layer mask info is not empty".to_owned());
    }
    section.finish()?;

    // E. Merged image.
    let (w, h) = (width as usize, height as usize);
    let compression = c.u16()?;
    let planes: Vec<Vec<u8>> = match compression {
        0 => (0..4).map(|_| read_plane_body(&mut c, 0, w, h)).collect::<Result<_, _>>()?,
        1 => {
            // All row lengths of all four planes come first.
            let lens: Vec<usize> = (0..4 * h).map(|_| c.u16().map(usize::from)).collect::<Result<_, _>>()?;
            lens.chunks(h).map(|plane_lens| read_rle_rows(&mut c, plane_lens, w)).collect::<Result<_, _>>()?
        }
        other => return Err(format!("merged image: unsupported compression {other}")),
    };
    c.finish()?;
    let mut merged = PixelBuf::new(width, height);
    for (channel, plane) in planes.iter().enumerate() {
        for (px, &v) in merged.data.as_chunks_mut::<4>().0.iter_mut().zip(plane) {
            px[channel] = v;
        }
    }
    Ok(PsdDoc { width, height, layers, merged })
}

/// A PSD as read back by `read_psd`.
#[doc(hidden)]
pub struct PsdDoc {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<PsdLayer>,
    pub merged: PixelBuf,
}

/// One layer of a `PsdDoc`.
#[doc(hidden)]
pub struct PsdLayer {
    pub rect: IRect,
    pub blend_key: [u8; 4],
    pub opacity: u8,
    pub hidden: bool,
    pub name: String,
    pub unicode_name: String,
    pub pixels: CroppedBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::BlendMode;
    use crate::testutil::XorShift;

    fn layer<'a>(name: &'a str, pixels: PixelView<'a>) -> ExportLayer<'a> {
        ExportLayer { name, kind: "paint", visible: true, opacity: 1.0, blend: BlendMode::Normal, pixels }
    }

    fn write(set: &ExportSet<'_>) -> Vec<u8> {
        let mut out = Vec::new();
        write_psd(&mut out, set).unwrap();
        out
    }

    fn be32(bytes: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    /// White-matted RGB with the real alpha, as the merged image stores it.
    fn matted(buf: &PixelBuf) -> PixelBuf {
        let mut out = buf.clone();
        if buf.data.as_chunks::<4>().0.iter().all(|p| p[3] == 255) {
            return out;
        }
        for px in out.data.as_chunks_mut::<4>().0 {
            let a = px[3] as f64;
            for c in &mut px[..3] {
                *c = ((*c as f64 * a + 255.0 * (255.0 - a)) / 255.0).round() as u8;
            }
        }
        out
    }

    #[test]
    fn golden_bytes_of_a_2x1_single_layer_file() {
        let pixels = PixelBuf::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        let set = ExportSet { name: "s", width: 2, height: 1, flattened: &pixels, layers: vec![layer("A", pixels.view())] };
        let bytes = write(&set);

        let mut expected: Vec<u8> = Vec::new();
        // A. Header.
        expected.extend_from_slice(b"8BPS");
        expected.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0, 0, 4]);
        expected.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 2, 0, 8, 0, 3]);
        // B. Colour mode data.
        expected.extend_from_slice(&[0, 0, 0, 0]);
        // C. Image resources: one resolution block.
        expected.extend_from_slice(&[0, 0, 0, 28]);
        expected.extend_from_slice(b"8BIM");
        expected.extend_from_slice(&[0x03, 0xED, 0, 0, 0, 0, 0, 16]);
        expected.extend_from_slice(&[0, 0x48, 0, 0, 0, 1, 0, 1, 0, 0x48, 0, 0, 0, 1, 0, 1]);
        // D. Layer and mask information: section length 128, layer info length 120, count -1.
        expected.extend_from_slice(&[0, 0, 0, 128, 0, 0, 0, 120, 0xFF, 0xFF]);
        // Record: top, left, bottom, right.
        expected.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2]);
        expected.extend_from_slice(&[0, 4]);
        expected.extend_from_slice(&[0xFF, 0xFF, 0, 0, 0, 7]);
        expected.extend_from_slice(&[0, 0, 0, 0, 0, 7]);
        expected.extend_from_slice(&[0, 1, 0, 0, 0, 6]);
        expected.extend_from_slice(&[0, 2, 0, 0, 0, 7]);
        expected.extend_from_slice(b"8BIMnorm");
        expected.extend_from_slice(&[255, 0, 0x08, 0]);
        // Extra data: 32 bytes = mask (4) + blending ranges (4) + Pascal name (4) + unicode name block (20).
        expected.extend_from_slice(&[0, 0, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0]);
        expected.extend_from_slice(&[1, b'A', 0, 0]);
        expected.extend_from_slice(b"8BIMluni");
        expected.extend_from_slice(&[0, 0, 0, 8, 0, 0, 0, 1, 0, 0x41, 0, 0]);
        // Channel data: compression 1, one row length, the PackBits row. Alpha, R, G, B.
        expected.extend_from_slice(&[0, 1, 0, 3, 1, 255, 128]);
        expected.extend_from_slice(&[0, 1, 0, 3, 1, 255, 0]);
        expected.extend_from_slice(&[0, 1, 0, 2, 255, 0]);
        expected.extend_from_slice(&[0, 1, 0, 3, 1, 0, 255]);
        // Padding of the layer info (119 -> 120) and the empty global layer mask info.
        expected.extend_from_slice(&[0, 0, 0, 0, 0]);
        // E. Merged image, RGB matted on white because one alpha is below 255: R, G, B, A.
        expected.extend_from_slice(&[0, 1, 0, 3, 0, 3, 0, 3, 0, 3]);
        expected.extend_from_slice(&[1, 255, 127, 1, 0, 127, 1, 0, 255, 1, 255, 128]);

        assert_eq!(bytes, expected);
        let doc = read_psd(&bytes).unwrap();
        assert_eq!((doc.width, doc.height, doc.layers.len()), (2, 1, 1));
        assert_eq!(doc.layers[0].pixels.data, pixels.data);
        assert_eq!(doc.merged.data, [255, 0, 0, 255, 127, 127, 255, 128]);
    }

    #[test]
    fn round_trip_of_three_layers() {
        let (w, h) = (37u32, 23u32);
        let mut rng = XorShift::new(11);
        // Bottom: opaque noise with runs. Middle: a cropped patch with partial alpha. Top: empty.
        let mut bottom = PixelBuf::new(w, h);
        for px in bottom.data.as_chunks_mut::<4>().0 {
            let v = (rng.next_u32() % 4 * 60) as u8;
            *px = [v, 255 - v, (rng.next_u32() % 2 * 255) as u8, 255];
        }
        let patch_rect = IRect::new(5, 3, 30, 20);
        let mut patch = CroppedBuf::new(patch_rect);
        for (i, px) in patch.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i as u32 % patch_rect.width(), i as u32 / patch_rect.width());
            // A transparent frame of 2 px makes the content bbox smaller than the cache rect.
            let inside = x >= 2 && y >= 2 && x < patch_rect.width() - 2 && y < patch_rect.height() - 2;
            *px = if inside { [rng.next_u32() as u8, 40, 200, 1 + (rng.next_u32() % 255) as u8] } else { [9, 9, 9, 0] };
        }
        let empty = PixelBuf::new(w, h);
        let mut flat = bottom.clone();
        flat.set(0, 0, [1, 2, 3, 0]);

        let name_mid = "Gr\u{fc}n \u{6728} \u{1f600}";
        let layers = vec![
            layer("Base", bottom.view()),
            ExportLayer {
                name: name_mid,
                kind: "stamp",
                visible: false,
                opacity: 0.5,
                blend: BlendMode::Multiply,
                pixels: patch.view(),
            },
            ExportLayer { name: "Empty", kind: "paint", visible: true, opacity: 0.25, blend: BlendMode::Add, pixels: empty.view() },
        ];
        let set = ExportSet { name: "set", width: w, height: h, flattened: &flat, layers };
        let bytes = write(&set);
        let doc = read_psd(&bytes).unwrap();
        assert_eq!((doc.width, doc.height), (w, h));
        assert_eq!(doc.layers.len(), 3);

        let l0 = &doc.layers[0];
        assert_eq!(l0.rect, IRect::new(0, 0, w as i32, h as i32));
        assert_eq!((l0.blend_key, l0.opacity, l0.hidden), (*b"norm", 255, false));
        assert_eq!((l0.name.as_str(), l0.unicode_name.as_str()), ("Base", "Base"));
        assert_eq!(l0.pixels.data, bottom.data);

        let l1 = &doc.layers[1];
        let content = IRect::new(7, 5, 28, 18);
        assert_eq!(l1.rect, content);
        assert_eq!((l1.blend_key, l1.opacity, l1.hidden), (*b"mul ", 128, true));
        assert_eq!(l1.name, "Gr?n ? ?");
        assert_eq!(l1.unicode_name, name_mid);
        assert_eq!(l1.pixels.rect, content);
        for y in content.y0..content.y1 {
            for x in content.x0..content.x1 {
                assert_eq!(l1.pixels.get(x, y), patch.get(x, y), "pixel {x},{y}");
            }
        }

        let l2 = &doc.layers[2];
        assert_eq!(l2.rect, IRect::new(0, 0, 0, 0));
        assert_eq!((l2.blend_key, l2.opacity, l2.hidden), (*b"lddg", 64, false));
        assert!(l2.pixels.is_empty());

        assert_eq!(doc.merged, matted(&flat));
        // The real alpha is kept, and with it the transparent pixel is white.
        assert_eq!(doc.merged.get(0, 0), [255, 255, 255, 0]);
    }

    #[test]
    fn opaque_merged_image_is_stored_unmatted() {
        let flat = PixelBuf::filled(5, 4, [10, 20, 30, 255]);
        let set = ExportSet { name: "s", width: 5, height: 4, flattened: &flat, layers: Vec::new() };
        let doc = read_psd(&write(&set)).unwrap();
        assert!(doc.layers.is_empty());
        assert_eq!(doc.merged, flat);
    }

    #[test]
    fn structure_lengths_are_consistent() {
        let mut rng = XorShift::new(3);
        for n_layers in 0..5usize {
            let (w, h) = (9 + n_layers as u32, 7u32);
            let bufs: Vec<PixelBuf> = (0..n_layers)
                .map(|i| {
                    let mut b = PixelBuf::new(w, h);
                    for px in b.data.as_chunks_mut::<4>().0.iter_mut().skip(i * 3) {
                        *px = [rng.next_u32() as u8, 1, 2, (rng.next_u32() % 3 * 100) as u8];
                    }
                    b
                })
                .collect();
            let names: Vec<String> = (0..n_layers).map(|i| "n".repeat(i)).collect();
            let layers: Vec<ExportLayer<'_>> = bufs.iter().zip(&names).map(|(b, n)| layer(n, b.view())).collect();
            let flat = PixelBuf::filled(w, h, [1, 2, 3, 200]);
            let bytes = write(&ExportSet { name: "s", width: w, height: h, flattened: &flat, layers });

            // The section starts after the header (26), colour mode data (4) and image resources (4 + 28).
            let section_at = 62;
            let section_len = be32(&bytes, section_at) as usize;
            let layer_info_len = be32(&bytes, section_at + 4) as usize;
            assert_eq!(section_len, 4 + layer_info_len + 4, "{n_layers} layers");
            assert_eq!(layer_info_len % 4, 0, "{n_layers} layers");
            let count = i16::from_be_bytes([bytes[section_at + 8], bytes[section_at + 9]]);
            assert_eq!(count, -(n_layers as i16));
            // The global layer mask info closes the section; the merged image follows.
            let merged_at = section_at + 4 + section_len;
            assert_eq!(be32(&bytes, merged_at - 4), 0);
            assert_eq!(&bytes[merged_at..merged_at + 2], [0, 1]);
            // Channel lengths: the strict reader slices every channel by its declared length.
            let doc = read_psd(&bytes).unwrap();
            assert_eq!(doc.layers.len(), n_layers);
            for (l, b) in doc.layers.iter().zip(&bufs) {
                for y in 0..h as i32 {
                    for x in 0..w as i32 {
                        let want = b.get(x as u32, y as u32);
                        if l.rect.contains(x, y) {
                            assert_eq!(l.pixels.get(x, y), want, "pixel {x},{y}");
                        } else {
                            // Only transparent pixels lie outside the content bbox.
                            assert_eq!(want[3], 0, "pixel {x},{y}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn strict_reader_rejects_damaged_files() {
        let pixels = PixelBuf::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        let set = ExportSet { name: "s", width: 2, height: 1, flattened: &pixels, layers: vec![layer("A", pixels.view())] };
        let bytes = write(&set);
        assert!(read_psd(&bytes).is_ok());
        // Trailing bytes.
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(read_psd(&longer).is_err());
        // Truncation anywhere.
        for cut in [0, 10, 26, 61, 70, 150, bytes.len() - 1] {
            assert!(read_psd(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        // A wrong section length.
        let mut bad = bytes.clone();
        bad[65] += 4;
        assert!(read_psd(&bad).is_err());
        // A wrong channel length (first channel of the first record: 7 -> 6).
        let mut bad = bytes.clone();
        assert_eq!(bad[72 + 16 + 2 + 5], 7);
        bad[72 + 16 + 2 + 5] = 6;
        assert!(read_psd(&bad).is_err());
        // A wrong signature.
        let mut bad = bytes.clone();
        bad[0] = b'9';
        assert!(read_psd(&bad).is_err());
        assert!(read_psd(&[]).is_err());
    }

    #[test]
    fn dimension_and_input_errors() {
        let tiny = PixelBuf::new(1, 1);
        for (width, height) in [(30001, 1), (1, 30001), (0, 1), (1, 0)] {
            let set = ExportSet { name: "s", width, height, flattened: &tiny, layers: Vec::new() };
            let mut out = Vec::new();
            match write_psd(&mut out, &set) {
                Err(ExportError::PsdDimension { width: w, height: h }) => assert_eq!((w, h), (width, height)),
                other => panic!("expected PsdDimension, got {other:?}"),
            }
            assert!(out.is_empty());
        }
        // 30000 is allowed (a 30000 x 1 strip is cheap).
        let strip = PixelBuf::filled(30000, 1, [5, 6, 7, 255]);
        let set = ExportSet { name: "s", width: 30000, height: 1, flattened: &strip, layers: vec![layer("strip", strip.view())] };
        let doc = read_psd(&write(&set)).unwrap();
        assert_eq!(doc.width, 30000);
        assert_eq!(doc.layers[0].pixels.data, strip.data);
        // A flattened image of the wrong size is refused.
        let set = ExportSet { name: "s", width: 2, height: 2, flattened: &tiny, layers: Vec::new() };
        assert!(matches!(write_psd(Vec::new(), &set), Err(ExportError::Io(_))));
    }

    #[test]
    fn names_are_padded_and_capped() {
        assert_eq!(pascal_name(""), [0, 0, 0, 0]);
        assert_eq!(pascal_name("abc"), [3, b'a', b'b', b'c']);
        assert_eq!(pascal_name("abcd"), [4, b'a', b'b', b'c', b'd', 0, 0, 0]);
        assert_eq!(pascal_name("\u{e4}b"), [2, b'?', b'b', 0]);
        let long = pascal_name(&"x".repeat(400));
        assert_eq!((long[0], long.len()), (255, 256));
        assert_eq!(unicode_name_block("").len(), 16);
        assert_eq!(unicode_name_block("A").len(), 20);
        assert_eq!(unicode_name_block("AB").len(), 20);
        assert_eq!(unicode_name_block("ABC").len(), 24);
        // A long name survives in the unicode block.
        let pixels = PixelBuf::filled(1, 1, [1, 2, 3, 255]);
        let name = "\u{6728}".repeat(300);
        let set = ExportSet { name: "s", width: 1, height: 1, flattened: &pixels, layers: vec![layer(&name, pixels.view())] };
        let doc = read_psd(&write(&set)).unwrap();
        assert_eq!(doc.layers[0].unicode_name, name);
        assert_eq!(doc.layers[0].name, "?".repeat(255));
    }

    #[test]
    fn layer_reaching_outside_the_canvas_is_clipped() {
        let mut patch = CroppedBuf::new(IRect::new(-2, -1, 3, 2));
        for px in patch.data.as_chunks_mut::<4>().0 {
            *px = [50, 60, 70, 255];
        }
        let flat = PixelBuf::new(4, 4);
        let set = ExportSet { name: "s", width: 4, height: 4, flattened: &flat, layers: vec![layer("clip", patch.view())] };
        let doc = read_psd(&write(&set)).unwrap();
        assert_eq!(doc.layers[0].rect, IRect::new(0, 0, 3, 2));
        assert!(doc.layers[0].pixels.data.as_chunks::<4>().0.iter().all(|p| *p == [50, 60, 70, 255]));
    }
}
