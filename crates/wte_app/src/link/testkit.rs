//! Making "the editor saved this" files for the link tests and the `sticker_link` scenario (stickers P9a): a read PSD
//! written back with the sticker writer, with one change (a repaint, a new layer, a removed layer). Nothing here starts
//! a program; the files stand for what PhotoCraft or Photoshop would save.

use std::path::Path;

use wte_core::psd::{PsdBlend, PsdDoc, PsdLimits, StickerPsd, StickerPsdLayer, write_sticker_to_vec};
use wte_core::{BlendMode, CroppedBuf, IRect};

/// The layers of a read PSD as the writer takes them (ids, names, pixels, masks, flags kept).
pub fn psd_of(doc: &PsdDoc) -> StickerPsd {
    let layers = doc
        .layers
        .iter()
        .enumerate()
        .map(|(i, l)| StickerPsdLayer {
            name: l.name.clone(),
            lyid: l.lyid.unwrap_or(1000 + i as u32),
            pixels: l.pixels.clone().unwrap_or_else(CroppedBuf::empty),
            mask: l.mask.clone(),
            clipped: l.clipped,
            visible: l.visible,
            opacity: l.opacity,
            blend: match l.blend {
                PsdBlend::Known(b) => b,
                PsdBlend::Other(_) => BlendMode::Normal,
            },
        })
        .collect::<Vec<_>>();
    let id_seed = layers.iter().map(|l| l.lyid).max().unwrap_or(0);
    StickerPsd { size: doc.size, layers, id_seed }
}

/// Reads the PSD at `path`.
pub fn read(path: &Path) -> Result<PsdDoc, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    wte_core::psd::read(&bytes, &PsdLimits::default()).map_err(|e| e.to_string())
}

/// A filled rectangle layer (straight RGBA) for a new layer of an edited file.
pub fn rect_layer(name: &str, lyid: u32, rect: IRect, rgba: [u8; 4]) -> StickerPsdLayer {
    let mut pixels = CroppedBuf::new(rect);
    pixels.data.as_chunks_mut::<4>().0.iter_mut().for_each(|p| *p = rgba);
    StickerPsdLayer { name: name.to_owned(), lyid, pixels, mask: None, clipped: false, visible: true, opacity: 255, blend: BlendMode::Normal }
}

/// Writes `psd` over `path` the way an editor's save lands: through a temp file and a rename.
pub fn save_as_editor(path: &Path, psd: &StickerPsd) -> Result<(), String> {
    let bytes = write_sticker_to_vec(psd).map_err(|e| e.to_string())?;
    save_bytes(path, &bytes)
}

/// Writes `bytes` over `path` through a temp file and a rename.
pub fn save_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("editor-tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// The file at `path` with one more layer on top: a red rectangle `rect` with id `lyid` (the next free one when the
/// file has it), named `name`.
pub fn with_new_layer(path: &Path, name: &str, lyid: u32, rect: IRect) -> Result<StickerPsd, String> {
    let mut psd = psd_of(&read(path)?);
    // An id the file already has becomes the next free one above them all (as an editor numbers a new layer).
    let top = psd.layers.iter().map(|l| l.lyid).max().unwrap_or(0);
    let lyid = if psd.layers.iter().any(|l| l.lyid == lyid) { top + 1 } else { lyid };
    psd.layers.push(rect_layer(name, lyid, rect, [220, 30, 40, 255]));
    psd.id_seed = psd.id_seed.max(lyid);
    Ok(psd)
}

/// A big-endian reader over a PSD's bytes (the splice below).
struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or("the file ends early")?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().map_err(|_| "u16")?))
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| "u32")?))
    }
}

/// Where the parts of a PSD's layer info are (version 1 files only).
struct LayerInfo {
    /// Start of the "layer and mask information" section (its length field).
    section: usize,
    /// End of that section.
    section_end: usize,
    /// End of the layer info (its padding included).
    info_end: usize,
    count: i16,
    /// Each layer record's byte range and the lengths of its channels' data.
    records: Vec<(std::ops::Range<usize>, Vec<usize>)>,
    /// Start of the channel image data (right after the last record).
    channels: usize,
}

fn layer_info(b: &[u8]) -> Result<LayerInfo, String> {
    let mut c = Cursor { b, at: 0 };
    if c.take(4)? != b"8BPS" || c.u16()? != 1 {
        return Err("not a version 1 PSD".into());
    }
    c.take(20)?;
    let n = c.u32()? as usize;
    c.take(n)?;
    let n = c.u32()? as usize;
    c.take(n)?;
    let section = c.at;
    let section_len = c.u32()? as usize;
    let section_end = section + 4 + section_len;
    let info_len = c.u32()? as usize;
    let info_end = c.at + info_len;
    let count = c.u16()? as i16;
    let mut records = Vec::new();
    for _ in 0..count.unsigned_abs() {
        let start = c.at;
        c.take(16)?;
        let channels = c.u16()? as usize;
        let mut lens = Vec::with_capacity(channels);
        for _ in 0..channels {
            c.take(2)?;
            lens.push(c.u32()? as usize);
        }
        c.take(12)?;
        let extra = c.u32()? as usize;
        c.take(extra)?;
        records.push((start..c.at, lens));
    }
    Ok(LayerInfo { section, section_end, info_end, count, records, channels: c.at })
}

/// The bytes of `base` with layer `index` of `donor` (an editor's own file, every tagged block of it kept: a type
/// layer's `TySh`, a smart object's `SoLd`) added on top, its `lyid` set to `lyid` (an editor numbers a new layer above
/// the ids it read). Both must be 8-bit version 1 PSDs; the base's merged picture stays as it is. This is "the editor
/// added its own layer and saved" with the editor's own bytes for that layer (the scenario's Photoshop pass, STICKER_PLAN
/// 13.9 "P9b": `layers\type-layer.psd` as the added text layer).
pub fn with_foreign_layer(base: &[u8], donor: &[u8], index: usize, lyid: u32) -> Result<Vec<u8>, String> {
    let bi = layer_info(base)?;
    let di = layer_info(donor)?;
    let (range, lens) = di.records.get(index).ok_or("the donor has no such layer")?;
    let mut record = donor[range.clone()].to_vec();
    let at = record.windows(8).position(|w| w == b"8BIMlyid").ok_or("the donor layer has no lyid")?;
    record[at + 12..at + 16].copy_from_slice(&lyid.to_be_bytes());
    let before: usize = di.records[..index].iter().flat_map(|(_, l)| l.iter()).sum();
    let data_start = di.channels + before;
    let data = donor.get(data_start..data_start + lens.iter().sum::<usize>()).ok_or("the donor's channel data ends early")?;
    let base_data: usize = bi.records.iter().flat_map(|(_, l)| l.iter()).sum();
    let mut info = Vec::new();
    let count = bi.count.unsigned_abs() as i16 + 1;
    info.extend_from_slice(&(if bi.count < 0 { -count } else { count }).to_be_bytes());
    info.extend_from_slice(&base[bi.records.first().map_or(bi.channels, |r| r.0.start)..bi.channels]);
    info.extend_from_slice(&record);
    info.extend_from_slice(&base[bi.channels..bi.channels + base_data]);
    info.extend_from_slice(data);
    while info.len() % 4 != 0 {
        info.push(0);
    }
    let rest = &base[bi.info_end..bi.section_end];
    let mut out = base[..bi.section].to_vec();
    out.extend_from_slice(&((4 + info.len() + rest.len()) as u32).to_be_bytes());
    out.extend_from_slice(&(info.len() as u32).to_be_bytes());
    out.extend_from_slice(&info);
    out.extend_from_slice(rest);
    out.extend_from_slice(&base[bi.section_end..]);
    Ok(out)
}

/// Stickers P12: sets `hasRealMergedData` of resource 1057 to false in a PSD the sticker writer made (what Photoshop or
/// the latest PhotoCraft write with "Maximize PSD and PSB File Compatibility" off: no usable merged picture).
pub fn clear_real_merged(bytes: &mut [u8]) -> Result<(), String> {
    let be32 = |b: &[u8], at: usize| -> Result<usize, String> {
        b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize).ok_or_else(|| "the file ends early".to_owned())
    };
    let colour = be32(bytes, 26)?;
    let start = 26 + 4 + colour + 4;
    let end = start + be32(bytes, 26 + 4 + colour)?;
    let mut at = start;
    while at + 12 <= end {
        let id = u16::from_be_bytes([bytes[at + 4], bytes[at + 5]]);
        let name_len = bytes[at + 6] as usize;
        let name = (1 + name_len).div_ceil(2) * 2;
        let size_at = at + 6 + name;
        let size = be32(bytes, size_at)?;
        let data = size_at + 4;
        if id == 1057 {
            let flag = bytes.get_mut(data + 4).ok_or("resource 1057 is too short")?;
            *flag = 0;
            return Ok(());
        }
        at = data + size.div_ceil(2) * 2;
    }
    Err("the file has no resource 1057".to_owned())
}
