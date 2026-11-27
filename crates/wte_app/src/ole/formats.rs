//! Stickers P10a (STICKER_PLAN 6.1, 6.2): pure codecs of the OLE data formats WTE reads and writes, with no OS call:
//! `CF_DIB` / `CF_DIBV5` <-> straight RGBA (`PixelBuf`), `DROPFILES` (the bytes of `CF_HDROP`), `FILEGROUPDESCRIPTORW`,
//! `CF_UNICODETEXT`, and WTE's private `WTE.Sticker.v1` wrapper (the process id of the WTE that made it, then the bytes
//! of a `.wtes`). Every parser takes bytes from another program: it never panics, never indexes past the end and
//! never multiplies lengths without a check; a malformed input is an `Err` or an empty list.
//!
//! Layouts (Windows SDK `wingdi.h`, `shlobj_core.h`, `oleidl.h`), all little-endian, written down here because the
//! tests assert them:
//! - `BITMAPINFOHEADER` 40 bytes: size u32 @0, width i32 @4, height i32 @8 (negative = top-down), planes u16 @12,
//!   bit count u16 @14, compression u32 @16, size image @20, x / y pels @24 / @28, colours used u32 @32, important @36.
//!   `BITMAPV5HEADER` 124 bytes adds the red / green / blue / alpha masks @40 / @44 / @48 / @52, the colour space @56.
//!   With `BI_BITFIELDS` (3) and a 40-byte header the three masks follow the header (`BI_ALPHABITFIELDS` (6): four).
//! - `DROPFILES` 20 bytes: offset of the list u32 @0, point @4, `fNC` @12, `fWide` @16; then the names, each ended by
//!   a NUL, and one more NUL.
//! - `FILEGROUPDESCRIPTORW`: count u32, then `FILEDESCRIPTORW` entries of 592 bytes: flags u32 @0, size high u32 @64,
//!   size low u32 @68, the name (260 UTF-16 units) @72. `FD_FILESIZE` = 0x40.

use std::path::PathBuf;

use wte_core::PixelBuf;

/// Standard clipboard formats (`winuser.h`).
pub const CF_DIB: u16 = 8;
pub const CF_UNICODETEXT: u16 = 13;
pub const CF_HDROP: u16 = 15;
pub const CF_DIBV5: u16 = 17;

/// Registered format names (`RegisterClipboardFormatW`).
pub const NAME_PNG: &str = "PNG";
pub const NAME_FILE_GROUP: &str = "FileGroupDescriptorW";
pub const NAME_FILE_CONTENTS: &str = "FileContents";
pub const NAME_STICKER: &str = "WTE.Sticker.v1";

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;
/// `LCS_sRGB` ('sRGB') and `LCS_GM_IMAGES` of the V5 header WTE writes.
const LCS_SRGB: u32 = 0x7352_4742;
const LCS_GM_IMAGES: u32 = 4;

/// Size of `DROPFILES`.
pub const DROPFILES_SIZE: usize = 20;
/// Size of one `FILEDESCRIPTORW` and the offsets inside it.
pub const FILE_DESCRIPTOR_SIZE: usize = 592;
const FD_NAME_AT: usize = 72;
const FD_SIZE_HIGH_AT: usize = 64;
const FD_SIZE_LOW_AT: usize = 68;
const FD_NAME_UNITS: usize = 260;
const FD_FILESIZE: u32 = 0x40;
/// Size of `BITMAPV5HEADER`.
pub const V5_HEADER_SIZE: usize = 124;

/// Largest side of a picture taken from a DIB (the stamp image limit of the core).
pub const MAX_DIB_SIDE: u32 = 16384;

/// Magic of `WTE.Sticker.v1`: "WTEs", then the version (1), the process id and the `.wtes` bytes.
const STICKER_MAGIC: &[u8; 4] = b"WTEs";
const STICKER_HEADER: usize = 12;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    u32_at(b, at).map(|v| v as i32)
}

/// One channel of a pixel by its mask, scaled to 8 bits (a mask of 0 gives `None`).
#[derive(Copy, Clone, Debug)]
struct Channel {
    shift: u32,
    bits: u32,
    mask: u32,
}

impl Channel {
    fn of(mask: u32) -> Option<Channel> {
        if mask == 0 {
            return None;
        }
        let shift = mask.trailing_zeros();
        let bits = (mask >> shift).trailing_ones().min(32 - shift);
        Some(Channel { shift, bits, mask })
    }

    fn get(self, v: u32) -> u8 {
        // Only the mask's lowest run of bits is the value (check P10a: a mask with gaps gave values above the
        // run's maximum, and `raw * 255` overflowed).
        let run = if self.bits >= 32 { u32::MAX } else { (1u32 << self.bits) - 1 };
        let raw = ((v & self.mask) >> self.shift) & run;
        match self.bits {
            0 => 0,
            8 => raw as u8,
            b if b > 8 => (raw >> (b - 8)) as u8,
            b => {
                let max = (1u32 << b) - 1;
                ((raw * 255 + max / 2) / max) as u8
            }
        }
    }
}

/// A `CF_DIB` / `CF_DIBV5` picture (a packed DIB: header, optional masks and colours, pixels) as straight RGBA.
///
/// Accepted: 24-bit `BI_RGB` (opaque), 32-bit `BI_RGB`, `BI_BITFIELDS` and `BI_ALPHABITFIELDS`, bottom-up or top-down.
/// Alpha (STICKER_PLAN 13.5): the alpha mask's bits (for 32-bit `BI_RGB` the fourth byte) are used as straight alpha,
/// **unless they are zero in every pixel**: many programs leave that byte empty, and the picture is then opaque. A
/// mask without alpha bits is opaque too. Data that looks premultiplied is left as it is. Everything else (palettes,
/// 16-bit, RLE, embedded JPEG / PNG) is an `Err` with a sentence.
pub fn dib_to_pixels(b: &[u8]) -> Result<PixelBuf, String> {
    let bad = |what: &str| Err(format!("the picture data is not a usable bitmap ({what})"));
    let Some(header) = u32_at(b, 0) else { return bad("too short") };
    let header = header as usize;
    if !(40..=b.len()).contains(&header) {
        return bad("header size");
    }
    let (Some(width), Some(height), Some(bpp), Some(compression), Some(colours)) =
        (i32_at(b, 4), i32_at(b, 8), u16_at(b, 14), u32_at(b, 16), u32_at(b, 32))
    else {
        return bad("too short");
    };
    if width <= 0 || height == 0 || height == i32::MIN {
        return bad("size");
    }
    let (w, h) = (width as u32, height.unsigned_abs());
    if w > MAX_DIB_SIDE || h > MAX_DIB_SIDE {
        return Err(format!("the picture is {w} x {h} pixels; at most {MAX_DIB_SIDE} a side can be used"));
    }
    let top_down = height < 0;
    // The masks: in the header (V2 and later), after a 40-byte header, or the fixed ones of BI_RGB.
    let mut offset = header;
    let masks: [u32; 4] = match (compression, bpp) {
        (BI_RGB, 24) => [0xFF_0000, 0xFF00, 0xFF, 0],
        (BI_RGB, 32) => [0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000],
        (BI_BITFIELDS | BI_ALPHABITFIELDS, 32) => {
            if header >= 52 {
                let alpha = if header >= 56 { u32_at(b, 52).unwrap_or(0) } else { 0 };
                [u32_at(b, 40).unwrap_or(0), u32_at(b, 44).unwrap_or(0), u32_at(b, 48).unwrap_or(0), alpha]
            } else {
                let n = if compression == BI_ALPHABITFIELDS { 4 } else { 3 };
                let mut m = [0u32; 4];
                for (i, slot) in m.iter_mut().enumerate().take(n) {
                    let Some(v) = u32_at(b, header + 4 * i) else { return bad("masks") };
                    *slot = v;
                }
                offset += 4 * n;
                m
            }
        }
        (BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS, _) => {
            return Err(format!("{bpp}-bit bitmaps are not supported; a 24- or 32-bit picture is needed"));
        }
        _ => return bad("compressed or embedded data"),
    };
    // Colours are listed for >= 16-bit pictures only as an optimisation hint; skip them.
    if colours > 256 {
        return bad("colour table");
    }
    offset += colours as usize * 4;
    let bytes_pp = u64::from(bpp / 8);
    let stride = (u64::from(w) * u64::from(bpp)).div_ceil(32) * 4;
    let need = stride * u64::from(h);
    let Some(avail) = (b.len() as u64).checked_sub(offset as u64) else { return bad("too short") };
    // Some writers put the three masks after a V5 header too: then exactly 12 more bytes than the pixels.
    if compression == BI_BITFIELDS && header > 40 && avail == need + 12 {
        offset += 12;
    } else if avail < need {
        return bad("too short");
    }
    let [rm, gm, bm, am] = masks;
    let (Some(r), Some(g), Some(bl)) = (Channel::of(rm), Channel::of(gm), Channel::of(bm)) else { return bad("masks") };
    let a = Channel::of(am);
    let (wu, hu, stride, bpp_b) = (w as usize, h as usize, stride as usize, bytes_pp as usize);
    let mut out = PixelBuf::new(w, h);
    let mut any_alpha = false;
    for y in 0..hu {
        let src_row = if top_down { y } else { hu - 1 - y };
        let row = &b[offset + src_row * stride..][..wu * bpp_b];
        let dst = &mut out.data[y * wu * 4..][..wu * 4];
        for (px, d) in row.chunks_exact(bpp_b).zip(dst.as_chunks_mut::<4>().0) {
            let v = match bpp_b {
                3 => u32::from(px[0]) | u32::from(px[1]) << 8 | u32::from(px[2]) << 16,
                _ => u32::from_le_bytes([px[0], px[1], px[2], px[3]]),
            };
            let alpha = a.map_or(0, |c| c.get(v));
            any_alpha |= alpha != 0;
            d.copy_from_slice(&[r.get(v), g.get(v), bl.get(v), alpha]);
        }
    }
    if !any_alpha {
        for d in out.data.as_chunks_mut::<4>().0 {
            d[3] = 255;
        }
    }
    Ok(out)
}

/// A straight-alpha picture as a `CF_DIBV5` packed DIB: `BITMAPV5HEADER` with `BI_BITFIELDS` and an alpha mask
/// (0xFF000000), sRGB, bottom-up, 32 bits per pixel (BGRA).
pub fn pixels_to_dibv5(p: &PixelBuf) -> Vec<u8> {
    let (w, h) = (p.width as usize, p.height as usize);
    let mut out = Vec::with_capacity(V5_HEADER_SIZE + w * h * 4);
    let push32 = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    push32(&mut out, V5_HEADER_SIZE as u32);
    push32(&mut out, p.width);
    push32(&mut out, p.height);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    push32(&mut out, BI_BITFIELDS);
    push32(&mut out, (w * h * 4) as u32);
    push32(&mut out, 2835);
    push32(&mut out, 2835);
    push32(&mut out, 0);
    push32(&mut out, 0);
    for mask in [0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000] {
        push32(&mut out, mask);
    }
    push32(&mut out, LCS_SRGB);
    out.extend_from_slice(&[0; 36 + 12]);
    push32(&mut out, LCS_GM_IMAGES);
    out.extend_from_slice(&[0; 12]);
    debug_assert_eq!(out.len(), V5_HEADER_SIZE);
    for y in (0..h).rev() {
        for px in p.data[y * w * 4..][..w * 4].as_chunks::<4>().0 {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    out
}

/// The bytes of a `CF_HDROP` (`DROPFILES` with wide names) listing `paths`.
pub fn build_dropfiles(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(DROPFILES_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    for path in paths {
        for unit in wide_of(path) {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(&[0, 0]);
    out
}

/// The paths of a `CF_HDROP` (`DROPFILES`): wide or ANSI names (ANSI read as Latin-1), at most `max` of them; a
/// malformed block gives the names that could be read before the fault.
pub fn parse_dropfiles(b: &[u8], max: usize) -> Vec<PathBuf> {
    let (Some(start), Some(wide)) = (u32_at(b, 0), u32_at(b, 16)) else { return Vec::new() };
    let Some(list) = b.get(start as usize..) else { return Vec::new() };
    let mut paths = Vec::new();
    if wide != 0 {
        let units: Vec<u16> = list.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        for name in units.split(|&u| u == 0) {
            if name.is_empty() || paths.len() >= max {
                break;
            }
            paths.push(path_of_wide(name));
        }
    } else {
        for name in list.split(|&c| c == 0) {
            if name.is_empty() || paths.len() >= max {
                break;
            }
            paths.push(PathBuf::from(name.iter().map(|&c| char::from(c)).collect::<String>()));
        }
    }
    paths
}

/// One entry of a `FILEGROUPDESCRIPTORW`: the virtual file's name and, when given, its size.
#[derive(Clone, Debug, PartialEq)]
pub struct FileDescriptor {
    pub name: String,
    pub size: Option<u64>,
}

/// The entries of a `FILEGROUPDESCRIPTORW` (at most as many as the bytes hold, and at most `max`).
pub fn parse_file_group(b: &[u8], max: usize) -> Vec<FileDescriptor> {
    let Some(count) = u32_at(b, 0) else { return Vec::new() };
    let fits = b.len().saturating_sub(4) / FILE_DESCRIPTOR_SIZE;
    let n = (count as usize).min(fits).min(max);
    (0..n)
        .map(|i| {
            let e = &b[4 + i * FILE_DESCRIPTOR_SIZE..][..FILE_DESCRIPTOR_SIZE];
            let flags = u32_at(e, 0).unwrap_or(0);
            let units: Vec<u16> = e[FD_NAME_AT..FD_NAME_AT + FD_NAME_UNITS * 2]
                .as_chunks::<2>().0.iter()
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&u| u != 0)
                .collect();
            let size = (flags & FD_FILESIZE != 0).then(|| {
                u64::from(u32_at(e, FD_SIZE_HIGH_AT).unwrap_or(0)) << 32 | u64::from(u32_at(e, FD_SIZE_LOW_AT).unwrap_or(0))
            });
            FileDescriptor { name: String::from_utf16_lossy(&units), size }
        })
        .collect()
}

/// A `FILEGROUPDESCRIPTORW` of `entries` (names cut to 259 units), for drag-out and the tests.
pub fn build_file_group(entries: &[FileDescriptor]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + entries.len() * FILE_DESCRIPTOR_SIZE);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        let mut d = vec![0u8; FILE_DESCRIPTOR_SIZE];
        if let Some(size) = e.size {
            d[0..4].copy_from_slice(&FD_FILESIZE.to_le_bytes());
            d[FD_SIZE_HIGH_AT..FD_SIZE_HIGH_AT + 4].copy_from_slice(&((size >> 32) as u32).to_le_bytes());
            d[FD_SIZE_LOW_AT..FD_SIZE_LOW_AT + 4].copy_from_slice(&(size as u32).to_le_bytes());
        }
        for (i, unit) in e.name.encode_utf16().take(FD_NAME_UNITS - 1).enumerate() {
            d[FD_NAME_AT + 2 * i..FD_NAME_AT + 2 * i + 2].copy_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&d);
    }
    out
}

/// The text of a `CF_UNICODETEXT` block (up to the first NUL; `\r\n` becomes `\n`).
pub fn text_from_unicode(b: &[u8]) -> String {
    let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units).replace("\r\n", "\n")
}

/// `text` as a `CF_UNICODETEXT` block (`\n` becomes `\r\n`, NUL-terminated).
pub fn text_to_unicode(text: &str) -> Vec<u8> {
    let windows = text.replace("\r\n", "\n").replace('\n', "\r\n");
    windows.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
}

/// The bytes of `WTE.Sticker.v1`: the magic, version 1, the process id of the WTE that offers it, then the `.wtes`.
pub fn wrap_sticker(process_id: u32, wtes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(STICKER_HEADER + wtes.len());
    out.extend_from_slice(STICKER_MAGIC);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&process_id.to_le_bytes());
    out.extend_from_slice(wtes);
    out
}

/// The process id and the `.wtes` bytes of a `WTE.Sticker.v1` block; `None` for another version or a bad block.
pub fn unwrap_sticker(b: &[u8]) -> Option<(u32, &[u8])> {
    if b.get(..4)? != STICKER_MAGIC || u32_at(b, 4)? != 1 {
        return None;
    }
    Some((u32_at(b, 8)?, b.get(STICKER_HEADER..)?))
}

/// The UTF-16 units of a path (lossless on Windows).
fn wide_of(path: &std::path::Path) -> Vec<u16> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().collect()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().encode_utf16().collect()
    }
}

/// A path from UTF-16 units (lossless on Windows, unpaired surrogates included).
fn path_of_wide(units: &[u16]) -> PathBuf {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_wide(units))
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(String::from_utf16_lossy(units))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A packed DIB with a header of `header` bytes (40 or 124), the given compression, bit count and masks, rows in
    /// file order.
    fn dib(header: usize, w: i32, h: i32, bpp: u16, compression: u32, masks: Option<[u32; 4]>, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut b = vec![0u8; header];
        b[0..4].copy_from_slice(&(header as u32).to_le_bytes());
        b[4..8].copy_from_slice(&w.to_le_bytes());
        b[8..12].copy_from_slice(&h.to_le_bytes());
        b[12..14].copy_from_slice(&1u16.to_le_bytes());
        b[14..16].copy_from_slice(&bpp.to_le_bytes());
        b[16..20].copy_from_slice(&compression.to_le_bytes());
        if let Some(m) = masks {
            if header >= 56 {
                for (i, v) in m.iter().enumerate() {
                    b[40 + 4 * i..44 + 4 * i].copy_from_slice(&v.to_le_bytes());
                }
            } else {
                for v in &m[..3] {
                    b.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        for row in rows {
            b.extend_from_slice(row);
            let pad = (4 - row.len() % 4) % 4;
            b.extend(std::iter::repeat_n(0, pad));
        }
        b
    }

    #[test]
    fn dib_24_bit_bottom_up_is_opaque() {
        // 2 x 2: file order is the bottom row first; BGR.
        let rows = vec![vec![255, 0, 0, 0, 255, 0], vec![0, 0, 255, 255, 255, 255]];
        let p = dib_to_pixels(&dib(40, 2, 2, 24, BI_RGB, None, &rows)).unwrap();
        assert_eq!((p.width, p.height), (2, 2));
        // Top row = the last row of the file: red, white.
        assert_eq!(&p.data[0..8], &[255, 0, 0, 255, 255, 255, 255, 255]);
        // Bottom row: blue, green.
        assert_eq!(&p.data[8..16], &[0, 0, 255, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn dib_32_bit_with_and_without_alpha() {
        // BI_RGB 32, top-down, one pixel with alpha 128 -> alpha is used.
        let rows = vec![vec![10, 20, 30, 128, 1, 2, 3, 0]];
        let p = dib_to_pixels(&dib(40, 2, -1, 32, BI_RGB, None, &rows)).unwrap();
        assert_eq!(p.data, vec![30, 20, 10, 128, 3, 2, 1, 0]);
        // Every alpha byte zero -> opaque (rev 3, 13.5).
        let rows = vec![vec![10, 20, 30, 0, 1, 2, 3, 0]];
        let p = dib_to_pixels(&dib(40, 2, -1, 32, BI_RGB, None, &rows)).unwrap();
        assert_eq!(p.data, vec![30, 20, 10, 255, 3, 2, 1, 255]);
    }

    #[test]
    fn dibv5_with_masks_round_trips_and_odd_masks_scale() {
        let mut p = PixelBuf::new(3, 2);
        for (i, v) in p.data.iter_mut().enumerate() {
            *v = (i * 37 % 256) as u8;
        }
        let bytes = pixels_to_dibv5(&p);
        assert_eq!(bytes.len(), V5_HEADER_SIZE + 3 * 2 * 4);
        assert_eq!(u32_at(&bytes, 0), Some(124));
        assert_eq!(u32_at(&bytes, 52), Some(0xFF00_0000), "alpha mask at 52");
        assert_eq!(dib_to_pixels(&bytes).unwrap(), p);
        // A 40-byte header with BI_BITFIELDS: masks follow the header; a 5-6-5-like mask in 32 bits scales to 8 bits.
        let masks = [0x0000_F800, 0x0000_07E0, 0x0000_001F, 0];
        let v: u32 = 0xF800 | 0x07E0; // red and green full, blue 0
        let rows = vec![v.to_le_bytes().to_vec()];
        let p = dib_to_pixels(&dib(40, 1, 1, 32, BI_BITFIELDS, Some(masks), &rows)).unwrap();
        assert_eq!(p.data, vec![255, 255, 0, 255]);
        // V5 with BI_BITFIELDS and no alpha mask: opaque.
        let rows = vec![vec![1, 2, 3, 99]];
        let p = dib_to_pixels(&dib(124, 1, 1, 32, BI_BITFIELDS, Some([0xFF_0000, 0xFF00, 0xFF, 0]), &rows)).unwrap();
        assert_eq!(p.data, vec![3, 2, 1, 255]);
        // Masks repeated after a V5 header (12 extra bytes) are skipped.
        let mut b = dib(124, 1, 1, 32, BI_BITFIELDS, Some([0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000]), &[]);
        for m in [0xFF_0000u32, 0xFF00, 0xFF] {
            b.extend_from_slice(&m.to_le_bytes());
        }
        b.extend_from_slice(&[5, 6, 7, 200]);
        assert_eq!(dib_to_pixels(&b).unwrap().data, vec![7, 6, 5, 200]);
    }

    #[test]
    fn malformed_dibs_never_panic() {
        assert!(dib_to_pixels(&[]).is_err());
        assert!(dib_to_pixels(&[40, 0, 0]).is_err());
        let good = pixels_to_dibv5(&PixelBuf::new(4, 4));
        for cut in 0..good.len() {
            let _ = dib_to_pixels(&good[..cut]);
        }
        // Huge sizes, palettes, odd bit counts, compressed data.
        let mut huge = good.clone();
        huge[4..8].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(dib_to_pixels(&huge).is_err());
        let mut min = good.clone();
        min[8..12].copy_from_slice(&i32::MIN.to_le_bytes());
        assert!(dib_to_pixels(&min).is_err());
        let mut pal = good.clone();
        pal[14..16].copy_from_slice(&8u16.to_le_bytes());
        assert!(dib_to_pixels(&pal).unwrap_err().contains("8-bit"));
        let mut jpeg = good.clone();
        jpeg[16..20].copy_from_slice(&4u32.to_le_bytes());
        assert!(dib_to_pixels(&jpeg).is_err());
        let mut colours = good.clone();
        colours[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(dib_to_pixels(&colours).is_err());
        // A seeded byte flip sweep.
        let mut seed = 0x1234_5678u32;
        for _ in 0..2000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let mut b = good.clone();
            let at = seed as usize % b.len();
            b[at] ^= (seed >> 24) as u8 | 1;
            let _ = dib_to_pixels(&b);
        }
    }

    /// Check P10a: masks with gaps (another program's bytes) never overflow or panic: only the mask's lowest run of
    /// bits is a channel's value, so a low-bit channel never multiplies a value larger than its maximum.
    #[test]
    fn check_p10a_masks_with_gaps_never_overflow() {
        // Red 0xFF00FF01 (lowest run: 1 bit), green 0x0000_00F0 (4 bits), blue 0x0F00_0000 (4 bits), no alpha.
        let masks = [0xFF00_FF01, 0x0000_00F0, 0x0F00_0000, 0];
        let rows = vec![u32::MAX.to_le_bytes().to_vec(), 0xFF00_FF00u32.to_le_bytes().to_vec()];
        let p = dib_to_pixels(&dib(40, 1, -2, 32, BI_BITFIELDS, Some(masks), &rows)).unwrap();
        // First pixel: every bit set -> each channel at its maximum; second (0xFF00FF00): red's run (bit 0) and
        // green's bits are clear, blue's are set; the bits of red's mask outside its run do not count.
        assert_eq!(p.data, vec![255, 255, 255, 255, 0, 0, 255, 255]);
        // Every single-run and gapped mask over every bit position.
        for shift in 0..32u32 {
            for mask in [1u32 << shift, u32::MAX << shift, (0x0101_0101u32).rotate_left(shift), (0x8000_0001u32).rotate_left(shift)] {
                let c = Channel::of(mask).unwrap();
                for v in [0, u32::MAX, mask, !mask, 0x5555_5555, 0xAAAA_AAAA] {
                    let _ = c.get(v);
                }
            }
        }
    }

    #[test]
    fn dropfiles_with_spaces_and_non_ascii_names() {
        let paths = vec![
            PathBuf::from(r"C:\My Pictures\logo final.png"),
            PathBuf::from(r"D:\Bilder\Größe – Ünïcödé 写真.jpg"),
            PathBuf::from(r"E:\a.wtes"),
        ];
        let b = build_dropfiles(&paths);
        assert_eq!(u32_at(&b, 0), Some(DROPFILES_SIZE as u32));
        assert_eq!(u32_at(&b, 16), Some(1), "wide names");
        assert_eq!(parse_dropfiles(&b, 64), paths);
        assert_eq!(parse_dropfiles(&b, 2), paths[..2]);
        // ANSI names.
        let mut ansi = vec![0u8; 20];
        ansi[0] = 20;
        ansi.extend_from_slice(b"C:\\x y.png\0C:\\z.bmp\0\0");
        assert_eq!(parse_dropfiles(&ansi, 64), [PathBuf::from(r"C:\x y.png"), PathBuf::from(r"C:\z.bmp")]);
        // Malformed: no panic, the names read before the fault.
        assert!(parse_dropfiles(&[], 64).is_empty());
        let mut bad = b.clone();
        bad[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_dropfiles(&bad, 64).is_empty());
        for cut in 0..b.len() {
            let got = parse_dropfiles(&b[..cut], 64);
            assert!(got.len() <= 3);
        }
    }

    #[test]
    fn file_group_descriptors() {
        let entries = vec![
            FileDescriptor { name: "photo (1).png".to_owned(), size: Some(5_000_000_000) },
            FileDescriptor { name: "Grüße.jpg".to_owned(), size: None },
        ];
        let b = build_file_group(&entries);
        assert_eq!(b.len(), 4 + 2 * FILE_DESCRIPTOR_SIZE);
        assert_eq!(parse_file_group(&b, 64), entries);
        assert_eq!(parse_file_group(&b, 1), entries[..1]);
        // A count larger than the bytes: only the entries that fit.
        let mut lying = b.clone();
        lying[0..4].copy_from_slice(&1000u32.to_le_bytes());
        assert_eq!(parse_file_group(&lying, 64).len(), 2);
        for cut in 0..b.len() {
            let _ = parse_file_group(&b[..cut], 64);
        }
        // A name of 260 units without a NUL stays inside its entry.
        let long = FileDescriptor { name: "x".repeat(400), size: None };
        let parsed = parse_file_group(&build_file_group(&[long]), 64);
        assert_eq!(parsed[0].name.len(), 259);
    }

    #[test]
    fn unicode_text_and_the_sticker_wrapper() {
        let b = text_to_unicode("a\nb – ü");
        assert_eq!(text_from_unicode(&b), "a\nb – ü");
        assert_eq!(&b[2..4], &[13, 0], "\\r before \\n");
        assert_eq!(text_from_unicode(&[0x41, 0]), "A", "an odd trailing byte is ignored");
        let w = wrap_sticker(4242, b"PK\x03\x04zip");
        assert_eq!(unwrap_sticker(&w), Some((4242, &b"PK\x03\x04zip"[..])));
        assert_eq!(unwrap_sticker(&w[..11]), None);
        let mut v2 = w.clone();
        v2[4] = 2;
        assert_eq!(unwrap_sticker(&v2), None);
        assert_eq!(unwrap_sticker(b"PK\x03\x04"), None);
    }
}
