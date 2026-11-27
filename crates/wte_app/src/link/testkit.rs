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
