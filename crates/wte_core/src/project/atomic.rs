// Owner: core-persist (C6). Atomic file replacement of ARCHITECTURE.md 5.23 / 7.1: temp file in the target's
// directory, fsync, verification, optional `.bak` rotation, rename.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// `<path>` with `suffix` appended to its file name (`model.wtep` -> `model.wtep.bak`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// The temp file `write_atomic` uses for `path`: `<file name>.tmp-<pid>` in the same directory.
pub(crate) fn temp_path_for(path: &Path) -> PathBuf {
    with_suffix(path, &format!(".tmp-{}", std::process::id()))
}

/// The backup `write_atomic` rotates an existing `path` to: `<path>.bak`.
pub(crate) fn backup_path_for(path: &Path) -> PathBuf {
    with_suffix(path, ".bak")
}

/// Calls `write` with a temp file next to `path`, fsyncs, runs `verify` on the temp path, optionally rotates the
/// existing file to `<path>.bak`, then renames. On any error the temp file is deleted and `path` is untouched.
///
/// The temp file is `<file name>.tmp-<pid>` in the directory of `path` (same volume, so the final rename is atomic
/// and replaces an existing file). `verify` runs after the temp file has been closed. With `keep_backup` an existing
/// `<path>.bak` is replaced; if the final rename fails after the rotation, the backup is moved back.
pub fn write_atomic<E: From<std::io::Error>>(
    path: &Path,
    keep_backup: bool,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), E>,
    verify: impl FnOnce(&Path) -> Result<(), E>,
) -> Result<(), E> {
    if path.file_name().is_none() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the target path has no file name").into());
    }
    let tmp = temp_path_for(path);
    let result = write_and_swap(path, &tmp, keep_backup, write, verify);
    if result.is_err() {
        // Best effort: the temp file may not exist (creation failed) or may already have been renamed.
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn write_and_swap<E: From<std::io::Error>>(
    path: &Path,
    tmp: &Path,
    keep_backup: bool,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), E>,
    verify: impl FnOnce(&Path) -> Result<(), E>,
) -> Result<(), E> {
    {
        let mut file = std::fs::File::create(tmp)?;
        write(&mut file)?;
        file.sync_all()?;
    }
    verify(tmp)?;

    let backup = backup_path_for(path);
    let rotated = keep_backup && path.is_file();
    if rotated {
        std::fs::rename(path, &backup)?;
    }
    if let Err(e) = std::fs::rename(tmp, path) {
        if rotated {
            // Put the original back so that `path` is untouched.
            let _ = std::fs::rename(&backup, path);
        }
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::error::ProjectError;
    use crate::testutil::temp_dir;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    fn put(path: &Path, keep_backup: bool, content: &[u8]) -> Result<(), ProjectError> {
        write_atomic(path, keep_backup, |f| Ok(f.write_all(content)?), |_| Ok(()))
    }

    #[test]
    fn writes_a_new_file_and_replaces_an_existing_one() {
        let dir = temp_dir("atomic-basic");
        let path = dir.join("doc.wtep");
        put(&path, false, b"first").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        put(&path, false, b"second version").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second version");
        assert_eq!(names(&dir), ["doc.wtep"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failing_writer_leaves_the_original_untouched_and_no_temp_file() {
        let dir = temp_dir("atomic-fail-write");
        let path = dir.join("doc.wtep");
        std::fs::write(&path, b"original").unwrap();
        let result: Result<(), ProjectError> = write_atomic(
            &path,
            true,
            |f| {
                f.write_all(b"half written")?;
                Err(ProjectError::Cancelled)
            },
            |_| panic!("verify must not run after a failed write"),
        );
        assert!(matches!(result, Err(ProjectError::Cancelled)));
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(names(&dir), ["doc.wtep"]);

        // Without an original nothing is left behind either.
        let fresh = dir.join("fresh.wtep");
        let result: Result<(), ProjectError> =
            write_atomic(&fresh, false, |_| Err(ProjectError::VerifyFailed("boom".into())), |_| Ok(()));
        assert!(matches!(result, Err(ProjectError::VerifyFailed(_))));
        assert_eq!(names(&dir), ["doc.wtep"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failing_verify_leaves_the_original_untouched_and_no_temp_file() {
        let dir = temp_dir("atomic-fail-verify");
        let path = dir.join("doc.wtep");
        std::fs::write(&path, b"original").unwrap();
        let tmp = temp_path_for(&path);
        let result: Result<(), ProjectError> = write_atomic(
            &path,
            true,
            |f| Ok(f.write_all(b"new content")?),
            |p| {
                // Verification sees the complete, closed temp file next to the target.
                assert_eq!(p, tmp);
                assert_eq!(std::fs::read(p).unwrap(), b"new content");
                Err(ProjectError::VerifyFailed("size mismatch".into()))
            },
        );
        assert!(matches!(result, Err(ProjectError::VerifyFailed(_))));
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(names(&dir), ["doc.wtep"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backup_rotation() {
        let dir = temp_dir("atomic-bak");
        let path = dir.join("doc.wtep");
        // Nothing to rotate on the first save.
        put(&path, true, b"v1").unwrap();
        assert_eq!(names(&dir), ["doc.wtep"]);
        put(&path, true, b"v2").unwrap();
        assert_eq!(names(&dir), ["doc.wtep", "doc.wtep.bak"]);
        assert_eq!(std::fs::read(dir.join("doc.wtep.bak")).unwrap(), b"v1");
        // An older backup is replaced.
        put(&path, true, b"v3").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"v3");
        assert_eq!(std::fs::read(dir.join("doc.wtep.bak")).unwrap(), b"v2");
        // Without keep_backup the backup is left alone.
        put(&path, false, b"v4").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"v4");
        assert_eq!(std::fs::read(dir.join("doc.wtep.bak")).unwrap(), b"v2");
        assert_eq!(names(&dir), ["doc.wtep", "doc.wtep.bak"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn io_errors_are_reported() {
        let dir = temp_dir("atomic-io");
        // The directory does not exist: creating the temp file fails.
        let path = dir.join("missing").join("doc.wtep");
        assert!(matches!(put(&path, false, b"x"), Err(ProjectError::Io(_))));
        // A path without a file name.
        assert!(matches!(put(Path::new(""), false, b"x"), Err(ProjectError::Io(_))));
        assert!(names(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn helper_paths() {
        let path = Path::new("some").join("dir").join("doc.wtep");
        assert_eq!(backup_path_for(&path), Path::new("some").join("dir").join("doc.wtep.bak"));
        let tmp = temp_path_for(&path);
        assert_eq!(tmp.parent(), path.parent());
        assert_eq!(tmp.file_name().unwrap().to_string_lossy(), format!("doc.wtep.tmp-{}", std::process::id()));
    }
}
