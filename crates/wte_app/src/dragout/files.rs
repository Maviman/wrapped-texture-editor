//! Stickers P11 (STICKER_PLAN 6.4 "Where the files go, and cleanup", review B7): the drag-out folder and its manifest.
//!
//! A drag out hands other programs a real file (`CF_HDROP`), so the picture is written to the drag-out folder
//! (`%LOCALAPPDATA%\WrappedTextureEditor\dragout\`; a self-test run: `<out_stem>_dragout`; tests: under `target\`).
//! PhotoCraft opens a dropped PNG **from this folder**, and its Save As may start here, so the user's own work can end up
//! in it. WTE therefore writes `manifest.json` beside its files - the name, size and modification time of every file
//! it wrote - and at start removes **only** files that are in the manifest, still have exactly that size and time, and
//! are older than one day. A file that is not in the manifest, or was changed since, is never touched (a changed one is
//! forgotten by the manifest, so it stays the user's); the folder itself is never removed. Linked PSDs (the PSD handle)
//! live in the project's links folder, never here.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The manifest's file name in the drag-out folder.
pub const MANIFEST: &str = "manifest.json";

/// How old a file of WTE's must be before the start-up cleanup removes it.
pub const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// One file WTE wrote: its name in the folder, size in bytes and modification time (nanoseconds since 1970).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub name: String,
    pub size: u64,
    pub modified_ns: u128,
}

/// `manifest.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub files: Vec<ManifestEntry>,
}

/// What the cleanup did (log line, tests).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cleanup {
    pub removed: Vec<String>,
    /// In the manifest and unchanged but not old enough.
    pub kept_fresh: Vec<String>,
    /// In the manifest but changed since WTE wrote it (or no longer there): forgotten, never touched.
    pub forgotten: Vec<String>,
}

/// The modification time of `meta` in nanoseconds since 1970 (0 when the system cannot tell).
fn modified_ns(meta: &std::fs::Metadata) -> u128 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos())
}

/// Reads the folder's manifest (an empty one when it is missing or unreadable: then nothing is ever removed).
pub fn read_manifest(folder: &Path) -> Manifest {
    std::fs::read(folder.join(MANIFEST)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Writes the manifest (a temporary file, then a rename).
fn write_manifest(folder: &Path, m: &Manifest) -> Result<(), String> {
    let tmp = folder.join("manifest.json.tmp");
    let bytes = serde_json::to_vec_pretty(m).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, folder.join(MANIFEST)).map_err(|e| format!("{}: {e}", folder.join(MANIFEST).display()))
}

/// The start-up cleanup of 6.4 (B7) at time `now`.
pub fn cleanup(folder: &Path, now: SystemTime) -> Cleanup {
    let mut out = Cleanup::default();
    let manifest = read_manifest(folder);
    if manifest.files.is_empty() {
        return out;
    }
    let now_ns = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut keep = Vec::new();
    for e in manifest.files {
        // Only a plain name inside the folder (a manifest edited by hand cannot point elsewhere).
        if e.name.is_empty() || Path::new(&e.name).components().count() != 1 || e.name == MANIFEST {
            out.forgotten.push(e.name);
            continue;
        }
        let path = folder.join(&e.name);
        let unchanged = std::fs::symlink_metadata(&path)
            .ok()
            .filter(|m| m.is_file())
            .is_some_and(|m| m.len() == e.size && modified_ns(&m) == e.modified_ns);
        if !unchanged {
            out.forgotten.push(e.name);
            continue;
        }
        if now_ns.saturating_sub(e.modified_ns) > MAX_AGE.as_nanos() {
            match std::fs::remove_file(&path) {
                Ok(()) => out.removed.push(e.name),
                Err(err) => {
                    log::warn!("drag out: {} could not be removed ({err}); kept", path.display());
                    out.kept_fresh.push(e.name.clone());
                    keep.push(e);
                }
            }
        } else {
            out.kept_fresh.push(e.name.clone());
            keep.push(e);
        }
    }
    if let Err(e) = write_manifest(folder, &Manifest { files: keep }) {
        log::warn!("drag out: the manifest could not be written ({e})");
    }
    out
}

/// A file name made of `name` (characters Windows refuses become `_`, at most 80 characters, "picture" when nothing
/// is left) with `ext`.
pub fn file_name(name: &str, ext: &str) -> String {
    let mut stem: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .take(80)
        .collect();
    stem = stem.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_owned();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.to_ascii_uppercase().as_str())
        || (stem.len() == 4 && ["COM", "LPT"].iter().any(|p| stem.to_ascii_uppercase().starts_with(p)) && stem.as_bytes()[3].is_ascii_digit());
    if stem.is_empty() || reserved {
        stem = format!("picture {stem}").trim().to_owned();
    }
    format!("{stem}.{ext}")
}

/// A name in `folder` that no file has yet: `name`, else "stem (2).ext", "stem (3).ext", ...
fn free_name(folder: &Path, name: &str) -> String {
    if !folder.join(name).exists() {
        return name.to_owned();
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..)
        .map(|n| if ext.is_empty() { format!("{stem} ({n})") } else { format!("{stem} ({n}).{ext}") })
        .find(|n| !folder.join(n).exists())
        .unwrap_or_else(|| name.to_owned())
}

/// Writes `bytes` as a new file named after `name` in `folder` (never over an existing file) and returns its path and
/// manifest entry (worker thread; the manifest itself is written by `record` on the UI thread).
pub fn write_new(folder: &Path, name: &str, bytes: &[u8]) -> Result<(PathBuf, ManifestEntry), String> {
    std::fs::create_dir_all(folder).map_err(|e| format!("{} could not be made: {e}", folder.display()))?;
    let name = free_name(folder, name);
    let path = folder.join(&name);
    // `create_new`: a file another program made meanwhile under that name is never overwritten.
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::io::Write::write_all(&mut file, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    drop(file);
    let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((path, ManifestEntry { name, size: meta.len(), modified_ns: modified_ns(&meta) }))
}

/// Adds a written file to the manifest.
pub fn record(folder: &Path, entry: ManifestEntry) {
    let mut m = read_manifest(folder);
    m.files.retain(|e| e.name != entry.name);
    m.files.push(entry);
    if let Err(e) = write_manifest(folder, &m) {
        log::warn!("drag out: the manifest could not be written ({e})");
    }
}

/// Removes a file of WTE's that nobody took (a drag that never left the window, or was cancelled): only when it is in
/// the manifest unchanged, or (`entry`) exactly as it was written; it is forgotten by the manifest either way.
pub fn remove_unused(folder: &Path, path: &Path, entry: Option<&ManifestEntry>) {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { return };
    if path.parent() != Some(folder) {
        return;
    }
    let mut m = read_manifest(folder);
    let listed = m.files.iter().find(|e| e.name == name).cloned();
    let expected = entry.cloned().or(listed.clone());
    let unchanged = expected.is_some_and(|e| {
        std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() == e.size && modified_ns(&meta) == e.modified_ns)
    });
    if unchanged && let Err(e) = std::fs::remove_file(path) {
        log::warn!("drag out: {} could not be removed ({e})", path.display());
    }
    if listed.is_some() {
        m.files.retain(|e| e.name != name);
        if let Err(e) = write_manifest(folder, &m) {
            log::warn!("drag out: the manifest could not be written ({e})");
        }
    }
}
