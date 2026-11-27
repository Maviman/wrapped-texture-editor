//! Stickers P10b (STICKER_PLAN 6.3, 13.5; ARCHITECTURE 8.17): the clipboard behind one trait, so no test and no
//! self-test run ever opens the Windows clipboard. `AppState::clipboard` is a [`MemClipboard`] in every self-test run
//! and in every test build; an interactive run on Windows gets the real one (`RealClipboard`, which is not even
//! compiled into a test build, like the editor link's `RealLauncher`).
//!
//! The formats WTE writes and reads (6.3): `WTE.Sticker.v1` (a `.wtes` with WTE's wrapper), the registered `PNG`,
//! `CF_DIBV5` (32-bit, straight alpha), `CF_DIB`, `CF_HDROP` (files) and `CF_UNICODETEXT`. Paste reads them in
//! [`PASTE_ORDER`] ([`paste_item`]); the codecs are those of the drop target (`formats.rs`), including the all-zero
//! alpha rule of 13.5. [`Clipboard::format_names`] lists everything the clipboard offers (standard formats by their
//! constant names, registered ones by their names): every Paste logs it, a paste that finds nothing usable says it,
//! and Help > Clipboard contents... shows it.
//!
//! The real clipboard (Windows): `OpenClipboard` with the app window as owner, retried 5 times 20 ms apart (another
//! program may hold it for a moment); a write empties it and hands each block to `SetClipboardData` (Windows owns the
//! block afterwards; a refused block is freed here); a read copies the block of `GetClipboardData` between
//! `GlobalLock` and `GlobalUnlock` and never frees it (the clipboard owns it). The clipboard is always closed again.

use std::cell::RefCell;

use super::DropItem;
use super::formats;

/// A clipboard format WTE writes or reads.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClipFormat {
    /// `WTE.Sticker.v1`: WTE's wrapper around the bytes of a `.wtes` (keeps a sticker editable, with its stamp styles).
    Sticker,
    /// The registered `PNG` (PhotoCraft and most programs read it first).
    Png,
    /// `CF_DIBV5`.
    DibV5,
    /// `CF_DIB`.
    Dib,
    /// `CF_HDROP`: files (`DROPFILES`).
    HDrop,
    /// `CF_UNICODETEXT`.
    UnicodeText,
}

impl ClipFormat {
    /// The format's name as `format_names` lists it.
    pub fn name(self) -> &'static str {
        match self {
            ClipFormat::Sticker => formats::NAME_STICKER,
            ClipFormat::Png => formats::NAME_PNG,
            ClipFormat::DibV5 => "CF_DIBV5",
            ClipFormat::Dib => "CF_DIB",
            ClipFormat::HDrop => "CF_HDROP",
            ClipFormat::UnicodeText => "CF_UNICODETEXT",
        }
    }
}

/// The order Paste reads the clipboard in (6.3): WTE's sticker first, then a PNG, a DIB, and the first image file.
pub const PASTE_ORDER: [ClipFormat; 5] = [ClipFormat::Sticker, ClipFormat::Png, ClipFormat::DibV5, ClipFormat::Dib, ClipFormat::HDrop];

/// The clipboard (`AppState::clipboard`). Errors are sentences for a toast.
pub trait Clipboard {
    /// Replaces the clipboard's content with `entries` (in this order).
    fn write(&mut self, entries: Vec<(ClipFormat, Vec<u8>)>) -> Result<(), String>;
    /// The bytes of `format`; `Ok(None)` when the clipboard does not offer it.
    fn read(&self, format: ClipFormat) -> Result<Option<Vec<u8>>, String>;
    /// The name of every format the clipboard offers, in its order.
    fn format_names(&self) -> Result<Vec<String>, String>;
    /// True for the in-memory clipboard (self-test runs, tests).
    fn is_memory(&self) -> bool;
}

/// The in-memory clipboard of self-test runs and tests: a list of named blocks. Unlike Windows it makes up no
/// format (no `CF_DIB` from a `CF_DIBV5`).
#[derive(Debug, Default)]
pub struct MemClipboard {
    entries: RefCell<Vec<(String, Vec<u8>)>>,
}

impl MemClipboard {
    /// Adds a block under any name (a test's foreign format, such as another program's private one).
    #[cfg_attr(not(test), allow(dead_code, reason = "tests put foreign formats on the clipboard"))]
    pub fn put_named(&mut self, name: &str, bytes: Vec<u8>) {
        self.entries.borrow_mut().push((name.to_owned(), bytes));
    }
}

impl Clipboard for MemClipboard {
    fn write(&mut self, entries: Vec<(ClipFormat, Vec<u8>)>) -> Result<(), String> {
        *self.entries.borrow_mut() = entries.into_iter().map(|(f, b)| (f.name().to_owned(), b)).collect();
        Ok(())
    }

    fn read(&self, format: ClipFormat) -> Result<Option<Vec<u8>>, String> {
        Ok(self.entries.borrow().iter().find(|(n, _)| n == format.name()).map(|(_, b)| b.clone()))
    }

    fn format_names(&self) -> Result<Vec<String>, String> {
        Ok(self.entries.borrow().iter().map(|(n, _)| n.clone()).collect())
    }

    fn is_memory(&self) -> bool {
        true
    }
}

/// The clipboard of a run: the in-memory one in a self-test run and in every test build, else (Windows) the real one.
pub fn clipboard_for(selftest: bool) -> Box<dyn Clipboard> {
    if !selftest && let Some(real) = real_clipboard() {
        return real;
    }
    Box::new(MemClipboard::default())
}

#[cfg(all(windows, not(test)))]
fn real_clipboard() -> Option<Box<dyn Clipboard>> {
    Some(Box::new(real::RealClipboard))
}

/// A test build (and a non-Windows one) has no real clipboard at all.
#[cfg(any(not(windows), test))]
fn real_clipboard() -> Option<Box<dyn Clipboard>> {
    None
}

/// The constant name of a standard clipboard format (`winuser.h`), `None` for a registered or private one.
pub fn standard_format_name(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "CF_TEXT",
        2 => "CF_BITMAP",
        3 => "CF_METAFILEPICT",
        4 => "CF_SYLK",
        5 => "CF_DIF",
        6 => "CF_TIFF",
        7 => "CF_OEMTEXT",
        8 => "CF_DIB",
        9 => "CF_PALETTE",
        10 => "CF_PENDATA",
        11 => "CF_RIFF",
        12 => "CF_WAVE",
        13 => "CF_UNICODETEXT",
        14 => "CF_ENHMETAFILE",
        15 => "CF_HDROP",
        16 => "CF_LOCALE",
        17 => "CF_DIBV5",
        0x80 => "CF_OWNERDISPLAY",
        0x81 => "CF_DSPTEXT",
        0x82 => "CF_DSPBITMAP",
        0x83 => "CF_DSPMETAFILEPICT",
        0x8E => "CF_DSPENHMETAFILE",
        _ => return None,
    })
}

/// The name of format `id` when it has no constant name and no registered name: private and GDI ranges, or a number.
pub fn unnamed_format(id: u32) -> String {
    match id {
        0x200..=0x2FF => format!("private format {id:#06x}"),
        0x300..=0x3FF => format!("GDI object format {id:#06x}"),
        _ => format!("format {id:#06x}"),
    }
}

/// The name a pasted picture without a file gets (its stamp is called "Pasted picture").
pub const PASTED_PICTURE: &str = "Pasted picture.png";
/// The name a pasted `.wtes` gets (its own sticker name is kept).
pub const PASTED_STICKER: &str = "Pasted sticker.wtes";

/// What Paste takes from the clipboard (6.3, read in `PASTE_ORDER`): WTE's sticker (from any WTE process, this one
/// included), else a PNG, a `CF_DIBV5`, a `CF_DIB`, else the first picture, PSD or sticker file of `CF_HDROP`.
/// `Ok(None)` when none of them is there (or none is usable: a bad sticker block or PNG falls through to the next
/// format). A DIB is not decoded here: that happens on the drop worker, with the all-zero alpha rule.
pub fn paste_item(cb: &dyn Clipboard) -> Result<Option<(ClipFormat, DropItem)>, String> {
    paste_item_in(cb, &PASTE_ORDER, crate::dnd::is_placeable_path)
}

/// Stickers P12: what Paste as New Paint Layer takes (5.9): a picture (PNG, CF_DIBV5, CF_DIB, then a picture or PSD file
/// of CF_HDROP). WTE's own sticker data is skipped: a WTE copy always carries its finished picture as PNG too.
pub const LAYER_PASTE_ORDER: [ClipFormat; 4] = [ClipFormat::Png, ClipFormat::DibV5, ClipFormat::Dib, ClipFormat::HDrop];

/// `paste_item` for Paste as New Paint Layer.
pub fn picture_item(cb: &dyn Clipboard) -> Result<Option<(ClipFormat, DropItem)>, String> {
    paste_item_in(cb, &LAYER_PASTE_ORDER, |p| crate::dnd::is_image_path(p) || crate::dnd::is_psd_path(p))
}

/// The first usable item of the formats `order` (a `CF_HDROP` file must pass `file_ok`).
fn paste_item_in(cb: &dyn Clipboard, order: &[ClipFormat], file_ok: fn(&std::path::Path) -> bool) -> Result<Option<(ClipFormat, DropItem)>, String> {
    use super::DataKind;
    for &format in order {
        let Some(bytes) = cb.read(format)? else { continue };
        let item = match format {
            ClipFormat::Sticker => formats::unwrap_sticker(&bytes)
                .map(|(_, wtes)| DropItem::Data { name: PASTED_STICKER.to_owned(), kind: DataKind::Sticker, bytes: wtes.to_vec() }),
            ClipFormat::Png => bytes
                .starts_with(b"\x89PNG")
                .then(|| DropItem::Data { name: PASTED_PICTURE.to_owned(), kind: DataKind::Png, bytes }),
            ClipFormat::DibV5 | ClipFormat::Dib => {
                (bytes.len() >= 40).then(|| DropItem::Data { name: PASTED_PICTURE.to_owned(), kind: DataKind::Dib, bytes })
            }
            ClipFormat::HDrop => formats::parse_dropfiles(&bytes, 64).into_iter().find(|p| file_ok(p)).map(DropItem::File),
            ClipFormat::UnicodeText => None,
        };
        if let Some(item) = item {
            return Ok(Some((format, item)));
        }
    }
    Ok(None)
}

/// The text of the clipboard (`CF_UNICODETEXT`, `\r\n` as `\n`); `Ok(None)` without text.
pub fn read_text(cb: &dyn Clipboard) -> Result<Option<String>, String> {
    Ok(cb.read(ClipFormat::UnicodeText)?.map(|b| formats::text_from_unicode(&b)))
}

/// `text` as the clipboard's only content.
pub fn write_text(cb: &mut dyn Clipboard, text: &str) -> Result<(), String> {
    cb.write(vec![(ClipFormat::UnicodeText, formats::text_to_unicode(text))])
}

/// The entries of a copied picture (6.3): `PNG` (already encoded), `CF_DIBV5` (32-bit, straight alpha) and, for a
/// sticker, `WTE.Sticker.v1` with this process's id.
pub fn picture_entries(png: Vec<u8>, pixels: &wte_core::PixelBuf, wtes: Option<&[u8]>) -> Vec<(ClipFormat, Vec<u8>)> {
    let mut out = vec![(ClipFormat::Png, png), (ClipFormat::DibV5, formats::pixels_to_dibv5(pixels))];
    if let Some(wtes) = wtes {
        out.push((ClipFormat::Sticker, formats::wrap_sticker(std::process::id(), wtes)));
    }
    out
}

/// The real clipboard (Windows, a non-test build only; constructed only by `clipboard_for(false)`).
#[cfg(all(windows, not(test)))]
mod real {
    #![allow(non_snake_case, reason = "Win32 names")]

    use super::{ClipFormat, Clipboard};
    use crate::ole::com::{self, Hglobal, Hwnd};

    #[link(name = "user32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn OpenClipboard(owner: Hwnd) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn GetClipboardData(format: u32) -> Hglobal;
        fn SetClipboardData(format: u32, mem: Hglobal) -> Hglobal;
        fn EnumClipboardFormats(format: u32) -> u32;
        fn GetClipboardFormatNameW(format: u32, name: *mut u16, max: i32) -> i32;
    }

    /// Tries to open the clipboard this many times, this far apart.
    const OPEN_TRIES: u32 = 5;
    const OPEN_WAIT: std::time::Duration = std::time::Duration::from_millis(20);

    /// The clipboard, open; closed when dropped.
    struct Open;

    impl Drop for Open {
        fn drop(&mut self) {
            // SAFETY: the clipboard was opened by `open` on this thread.
            unsafe {
                CloseClipboard();
            }
        }
    }

    fn open() -> Result<Open, String> {
        let owner = crate::ole::window().unwrap_or(0);
        for i in 0..OPEN_TRIES {
            // SAFETY: plain user32 call; `owner` is the app's window or 0.
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(Open);
            }
            if i + 1 < OPEN_TRIES {
                std::thread::sleep(OPEN_WAIT);
            }
        }
        Err("the clipboard is in use by another program; try again".to_owned())
    }

    fn id_of(format: ClipFormat) -> u32 {
        let f = crate::ole::drop_target::Formats::get();
        u32::from(match format {
            ClipFormat::Sticker => f.sticker,
            ClipFormat::Png => f.png,
            ClipFormat::DibV5 => crate::ole::formats::CF_DIBV5,
            ClipFormat::Dib => crate::ole::formats::CF_DIB,
            ClipFormat::HDrop => crate::ole::formats::CF_HDROP,
            ClipFormat::UnicodeText => crate::ole::formats::CF_UNICODETEXT,
        })
    }

    /// The Windows clipboard.
    pub struct RealClipboard;

    impl Clipboard for RealClipboard {
        fn write(&mut self, entries: Vec<(ClipFormat, Vec<u8>)>) -> Result<(), String> {
            if crate::ole::window().is_none() {
                return Err("the clipboard needs the app window, which is not known".to_owned());
            }
            let _open = open()?;
            // SAFETY: the clipboard is open on this thread with the app window as owner.
            if unsafe { EmptyClipboard() } == 0 {
                return Err("the clipboard could not be emptied".to_owned());
            }
            for (format, bytes) in entries {
                let id = id_of(format);
                if id == 0 {
                    continue;
                }
                let mem = com::global_from(&bytes);
                if mem == 0 {
                    return Err("not enough memory for the clipboard".to_owned());
                }
                // SAFETY: `mem` is a movable block we own; on success Windows owns it, on failure we free it.
                unsafe {
                    if SetClipboardData(id, mem) == 0 {
                        com::GlobalFree(mem);
                        return Err(format!("the clipboard did not take the {} data", format.name()));
                    }
                }
            }
            Ok(())
        }

        fn read(&self, format: ClipFormat) -> Result<Option<Vec<u8>>, String> {
            let id = id_of(format);
            if id == 0 {
                return Ok(None);
            }
            let _open = open()?;
            // SAFETY: the clipboard is open on this thread; the block belongs to the clipboard and is only read
            // between `GlobalLock` and `GlobalUnlock` (never freed here).
            unsafe {
                let mem = GetClipboardData(id);
                if mem == 0 {
                    return Ok(None);
                }
                let size = com::GlobalSize(mem);
                let p = com::GlobalLock(mem);
                if p.is_null() {
                    return Ok(None);
                }
                let bytes = std::slice::from_raw_parts(p.cast::<u8>(), size).to_vec();
                com::GlobalUnlock(mem);
                Ok(Some(bytes))
            }
        }

        fn format_names(&self) -> Result<Vec<String>, String> {
            let _open = open()?;
            let mut names = Vec::new();
            let mut id = 0;
            // SAFETY: the clipboard is open on this thread; the name buffer is 256 units long and the call writes at
            // most `max` units including the NUL.
            unsafe {
                loop {
                    id = EnumClipboardFormats(id);
                    if id == 0 || names.len() >= 256 {
                        break;
                    }
                    let name = match super::standard_format_name(id) {
                        Some(n) => n.to_owned(),
                        None => {
                            let mut buf = [0u16; 256];
                            let n = GetClipboardFormatNameW(id, buf.as_mut_ptr(), buf.len() as i32);
                            if n > 0 {
                                String::from_utf16_lossy(&buf[..n as usize])
                            } else {
                                super::unnamed_format(id)
                            }
                        }
                    };
                    names.push(name);
                }
            }
            Ok(names)
        }

        fn is_memory(&self) -> bool {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ole::DataKind;
    use wte_core::PixelBuf;

    fn picture(w: u32, h: u32) -> PixelBuf {
        let mut p = PixelBuf::new(w, h);
        for (i, px) in p.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *px = [(i * 40 % 256) as u8, 100, 200, (i * 70 % 256) as u8];
        }
        p
    }

    #[test]
    fn the_test_build_has_only_the_in_memory_clipboard() {
        assert!(clipboard_for(false).is_memory(), "a test build never gets the real clipboard");
        assert!(clipboard_for(true).is_memory());
    }

    /// In-memory round trip of a picture (alpha kept) and of a `.wtes`, read in the paste order.
    #[test]
    fn pictures_and_stickers_round_trip_with_their_alpha() {
        let mut cb = MemClipboard::default();
        let p = picture(5, 3);
        let png = wte_core::encode_png(5, 3, &p.data, wte_core::PngLevel::Fast).unwrap();
        cb.write(picture_entries(png.clone(), &p, Some(b"PK-wtes-bytes"))).unwrap();
        assert_eq!(cb.format_names().unwrap(), ["PNG", "CF_DIBV5", "WTE.Sticker.v1"]);
        // The sticker is read first (even from this very process), and comes without the wrapper.
        let (format, item) = paste_item(&cb).unwrap().unwrap();
        assert_eq!(format, ClipFormat::Sticker);
        assert_eq!(item, DropItem::Data { name: PASTED_STICKER.into(), kind: DataKind::Sticker, bytes: b"PK-wtes-bytes".to_vec() });
        // Without it: the PNG, whose pixels (alpha included) are the copied ones.
        cb.write(picture_entries(png.clone(), &p, None)).unwrap();
        let (format, item) = paste_item(&cb).unwrap().unwrap();
        assert_eq!(format, ClipFormat::Png);
        let DropItem::Data { bytes, kind: DataKind::Png, .. } = item else { panic!("{item:?}") };
        assert_eq!(wte_core::decode_image(&bytes, Some("x.png"), 64).unwrap().pixels, p);
        // Only the DIBV5: the same pixels through the DIB codec.
        let dib = formats::pixels_to_dibv5(&p);
        cb.write(vec![(ClipFormat::DibV5, dib.clone())]).unwrap();
        let (format, item) = paste_item(&cb).unwrap().unwrap();
        assert_eq!(format, ClipFormat::DibV5);
        let DropItem::Data { bytes, kind: DataKind::Dib, .. } = item else { panic!("{item:?}") };
        assert_eq!(formats::dib_to_pixels(&bytes).unwrap(), p);
    }

    /// 13.5: a 32-bit DIB whose alpha is zero everywhere is opaque; one with mixed alpha keeps it (the clipboard path).
    #[test]
    fn a_32_bit_dib_with_zero_alpha_is_opaque_and_mixed_alpha_is_kept() {
        let mut p = picture(4, 2);
        for px in p.data.as_chunks_mut::<4>().0 {
            px[3] = 0;
        }
        let mut cb = MemClipboard::default();
        cb.write(vec![(ClipFormat::DibV5, formats::pixels_to_dibv5(&p))]).unwrap();
        let (_, DropItem::Data { bytes, .. }) = paste_item(&cb).unwrap().unwrap() else { panic!() };
        let read = formats::dib_to_pixels(&bytes).unwrap();
        assert!(read.data.as_chunks::<4>().0.iter().all(|px| px[3] == 255), "zero alpha everywhere = opaque");
        let mixed = picture(4, 2);
        assert!(mixed.data.as_chunks::<4>().0.iter().any(|px| px[3] == 0) && mixed.data.as_chunks::<4>().0.iter().any(|px| px[3] > 0));
        cb.write(vec![(ClipFormat::Dib, formats::pixels_to_dibv5(&mixed))]).unwrap();
        let (format, DropItem::Data { bytes, .. }) = paste_item(&cb).unwrap().unwrap() else { panic!() };
        assert_eq!(format, ClipFormat::Dib);
        assert_eq!(formats::dib_to_pixels(&bytes).unwrap(), mixed, "mixed alpha is used as straight alpha");
    }

    #[test]
    fn files_text_and_foreign_formats() {
        let mut cb = MemClipboard::default();
        // CF_HDROP: the first picture, PSD or sticker file (a model is not pasted).
        let files = [std::path::PathBuf::from(r"C:\m\model.obj"), std::path::PathBuf::from(r"C:\Bilder\Größe 1.PNG")];
        cb.write(vec![(ClipFormat::HDrop, formats::build_dropfiles(&files))]).unwrap();
        assert_eq!(paste_item(&cb).unwrap().unwrap().1, DropItem::File(files[1].clone()));
        cb.write(vec![(ClipFormat::HDrop, formats::build_dropfiles(&files[..1]))]).unwrap();
        assert_eq!(paste_item(&cb).unwrap(), None);
        // A bad sticker block or a PNG that is not one falls through to the next format.
        cb.write(vec![(ClipFormat::Sticker, b"junk".to_vec()), (ClipFormat::Png, b"no png".to_vec())]).unwrap();
        assert_eq!(paste_item(&cb).unwrap(), None);
        // Text: \r\n becomes \n both ways.
        write_text(&mut cb, "one\ntwo").unwrap();
        assert_eq!(cb.read(ClipFormat::UnicodeText).unwrap().unwrap(), formats::text_to_unicode("one\r\ntwo"));
        assert_eq!(read_text(&cb).unwrap().as_deref(), Some("one\ntwo"));
        assert_eq!(paste_item(&cb).unwrap(), None, "text is not a picture");
        // Another program's format is listed by its name.
        cb.put_named("Photoshop Clipboard", vec![1, 2, 3]);
        assert_eq!(cb.format_names().unwrap(), ["CF_UNICODETEXT", "Photoshop Clipboard"]);
    }

    #[test]
    fn format_names_of_standard_and_unnamed_formats() {
        assert_eq!(standard_format_name(8), Some("CF_DIB"));
        assert_eq!(standard_format_name(17), Some("CF_DIBV5"));
        assert_eq!(standard_format_name(15), Some("CF_HDROP"));
        assert_eq!(standard_format_name(13), Some("CF_UNICODETEXT"));
        assert_eq!(standard_format_name(2), Some("CF_BITMAP"));
        assert_eq!(standard_format_name(0xC123), None);
        assert_eq!(unnamed_format(0x201), "private format 0x0201");
        assert_eq!(unnamed_format(0x305), "GDI object format 0x0305");
        for f in PASTE_ORDER {
            assert!(!f.name().is_empty());
        }
    }
}
