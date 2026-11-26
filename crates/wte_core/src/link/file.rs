//! Writing a link file (STICKER_PLAN 5.3 "Who writes the PSD", 13.1 A8).
//!
//! Only three things write a link file: making the link, an explicit "Send to <editor>", and "Edit in <editor>" when
//! WTE holds newer content. A project save never does (the save code has no writer for link files at all). Every
//! write is atomic: the bytes go to a temp file in the same folder, the old file is kept as `<name>.bak.psd` (one
//! generation), the new content's CRC-32 is registered with the watcher **before** the rename (`own_writes`, 5.6),
//! and the temp file is renamed over the PSD.
//!
//! An editor may hold the PSD open so that the rename fails (assumption A8). Then the rename is tried three more
//! times 200 ms apart; if it still fails, the temp files are deleted and **nothing** is changed: the PSD, its
//! `.bak.psd`, the link's `synced` (the caller does not call `finish_link_write`) and the registration
//! (`LinkFileIo::rename_failed`). The result says "could not be replaced" ([`LinkWriteError::sentence`]). The rename
//! and the pause are injected ([`LinkFileIo`]), so the failure path is tested without a locked file.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::FileStamp;
use super::texts;

/// More tries of a refused rename (13.1: three more, 200 ms apart).
pub const RENAME_RETRIES: u32 = 3;
/// The pause between two tries.
pub const RENAME_PAUSE_MS: u64 = 200;

/// The steps of a link write that the tests replace.
pub trait LinkFileIo {
    /// Renames `from` over `to` (replacing it).
    fn rename(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }

    /// Waits `ms` milliseconds before the next try.
    fn pause(&mut self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    /// Called with the new content's CRC-32 right before the first rename (the watcher's `own_writes`, 5.6).
    fn before_rename(&mut self, _crc32: u32) {}

    /// Called when the write gave up: the registration of `before_rename` is taken back.
    fn rename_failed(&mut self, _crc32: u32) {}
}

/// The real file system, no watcher.
#[derive(Copy, Clone, Debug, Default)]
pub struct RealFileIo;

impl LinkFileIo for RealFileIo {}

/// Why a link write failed. Nothing was changed in either case.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LinkWriteError {
    /// The rename was refused (sharing or access error) after every retry: another program has the file open.
    #[error("{file} could not be replaced because another program has it open")]
    Locked { file: String },
    /// Writing the temp file failed (disk full, no folder, ...).
    #[error("{file} could not be written: {message}")]
    Io { file: String, message: String },
}

impl LinkWriteError {
    /// The in-window message (13.1 A8), with the link's editor name.
    pub fn sentence(&self, editor: &str) -> String {
        match self {
            LinkWriteError::Locked { file } => texts::could_not_replace(file, editor),
            LinkWriteError::Io { file, message } => format!("{file} could not be written: {message}."),
        }
    }
}

/// `<dir>/<stem>.bak.psd` for `<dir>/<stem>.psd` (any other name gets `.bak.psd` appended to its stem).
pub fn backup_path(target: &Path) -> PathBuf {
    let stem = target.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    target.with_file_name(format!("{stem}.bak.psd"))
}

/// A refused rename: Windows sharing violation (32), lock violation (33), access denied (5), or `PermissionDenied`.
fn is_lock_error(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5 | 32 | 33)) || e.kind() == std::io::ErrorKind::PermissionDenied
}

/// Writes `bytes` to `target` atomically (module docs) and returns the new file's stamp. The folder must exist.
pub fn write_link_file(target: &Path, bytes: &[u8], io: &mut dyn LinkFileIo) -> Result<FileStamp, LinkWriteError> {
    let file = target.file_name().map_or_else(|| target.display().to_string(), |n| n.to_string_lossy().into_owned());
    let io_err = |e: std::io::Error| LinkWriteError::Io { file: file.clone(), message: e.to_string() };
    let tmp = target.with_file_name(format!("{file}.wte-tmp"));
    let bak = backup_path(target);
    let bak_tmp = bak.with_file_name(format!("{}.wte-tmp", bak.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())));
    let cleanup = |paths: &[&PathBuf]| {
        for p in paths {
            let _ = std::fs::remove_file(p);
        }
    };
    let written = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    if let Err(e) = written {
        cleanup(&[&tmp]);
        return Err(io_err(e));
    }
    // The old file becomes the backup only once the new one is in place: copy it aside first.
    let had_old = target.exists();
    if had_old && let Err(e) = std::fs::copy(target, &bak_tmp) {
        cleanup(&[&tmp, &bak_tmp]);
        return Err(io_err(e));
    }
    let crc = crc32fast::hash(bytes);
    io.before_rename(crc);
    let mut tries = 0;
    loop {
        match io.rename(&tmp, target) {
            Ok(()) => break,
            Err(e) if is_lock_error(&e) && tries < RENAME_RETRIES => {
                tries += 1;
                io.pause(RENAME_PAUSE_MS);
            }
            Err(e) => {
                cleanup(&[&tmp, &bak_tmp]);
                io.rename_failed(crc);
                return Err(if is_lock_error(&e) { LinkWriteError::Locked { file: file.clone() } } else { io_err(e) });
            }
        }
    }
    if had_old && std::fs::rename(&bak_tmp, &bak).is_err() {
        // The new file is in place; a backup that cannot be kept is not a failure of the write.
        log::warn!("{}: the backup copy could not be kept", bak.display());
        cleanup(&[&bak_tmp]);
    }
    let modified = std::fs::metadata(target).ok().and_then(|m| m.modified().ok());
    Ok(FileStamp::of_bytes(bytes, modified))
}
