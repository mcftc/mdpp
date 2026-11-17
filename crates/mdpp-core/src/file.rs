//! Saving without losing data (PLAN §3.3).
//!
//! The normal path is atomic: write a temporary file next to the target, flush it to disk, then
//! rename it over the target — a crash leaves either the old or the new file, never half of one.
//! Where an atomic replace would change something the user did not ask to change, we write in
//! place instead (the hot-exit backup, Phase 1, is the safety net then):
//! * a **symlink** is resolved and its *target* is replaced, so the link survives;
//! * a **hard-linked** file is written in place, so every link keeps seeing the same file;
//! * a **read-only directory** (no temp file possible) is written in place.
//!
//! Unix permission bits are copied to the replacement. On Windows, `rename` replaces the file;
//! `ReplaceFileW` (keeps ACLs, survives scanners holding the file) is a planned improvement.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// How a save was carried out — useful in logs and tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveMethod {
    /// Temporary file + rename.
    Atomic,
    /// Truncate-and-write of the existing file (hard links, read-only directories, failed rename).
    InPlace,
}

/// Write `bytes` to `path`. Returns where the data went (the resolved target) and how.
pub fn save(path: &Path, bytes: &[u8]) -> io::Result<(PathBuf, SaveMethod)> {
    let target = resolve_symlinks(path)?;
    let existing = fs::metadata(&target).ok();
    if existing.as_ref().is_some_and(has_other_hard_links) {
        write_in_place(&target, bytes)?;
        return Ok((target, SaveMethod::InPlace));
    }

    let dir = target
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (temp_path, mut temp) = match create_temp_beside(&target, dir) {
        Ok(created) => created,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            write_in_place(&target, bytes)?;
            return Ok((target, SaveMethod::InPlace));
        }
        Err(error) => return Err(error),
    };

    let written = (|| {
        temp.write_all(bytes)?;
        temp.sync_all()?;
        if let Some(meta) = &existing {
            fs::set_permissions(&temp_path, meta.permissions())?;
        }
        drop(temp);
        fs::rename(&temp_path, &target)
    })();
    match written {
        Ok(()) => {
            sync_dir(dir);
            Ok((target, SaveMethod::Atomic))
        }
        Err(_) => {
            // The original is untouched at this point; fall back to writing it in place.
            let _ = fs::remove_file(&temp_path);
            write_in_place(&target, bytes)?;
            Ok((target, SaveMethod::InPlace))
        }
    }
}

/// Follow a chain of symlinks to the file that really holds the data. A dangling link resolves
/// to the path it points at, so saving creates that file.
fn resolve_symlinks(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    // Bounded like the OS's own loop detection.
    for _ in 0..40 {
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let link = fs::read_link(&current)?;
                current = match current.parent() {
                    Some(parent) if link.is_relative() => parent.join(link),
                    _ => link,
                };
            }
            Ok(_) => return Ok(current),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other(format!(
        "too many levels of symbolic links: {}",
        path.display()
    )))
}

#[cfg(unix)]
fn has_other_hard_links(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.nlink() > 1
}

#[cfg(not(unix))]
fn has_other_hard_links(_meta: &fs::Metadata) -> bool {
    false
}

fn create_temp_beside(target: &Path, dir: &Path) -> io::Result<(PathBuf, File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = target
        .file_name()
        .map_or_else(|| "file".into(), |n| n.to_string_lossy());
    loop {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!(".{name}.mdpp-{}-{unique}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

fn write_in_place(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(target)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Make the rename itself durable (Unix); best effort.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the system temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "mdpp-test-{name}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn creates_and_replaces_atomically() {
        let dir = TempDir::new("atomic");
        let path = dir.0.join("a.md");
        assert_eq!(
            save(&path, b"one").unwrap(),
            (path.clone(), SaveMethod::Atomic)
        );
        assert_eq!(save(&path, b"two").unwrap().1, SaveMethod::Atomic);
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert!(leftovers(&dir.0).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("perm");
        let path = dir.0.join("run.md");
        fs::write(&path, "x").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
        save(&path, b"y").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_survives_and_target_changes() {
        let dir = TempDir::new("link");
        let real = dir.0.join("real.md");
        let link = dir.0.join("link.md");
        fs::write(&real, "old").unwrap();
        std::os::unix::fs::symlink("real.md", &link).unwrap();
        let (written, method) = save(&link, b"new").unwrap();
        assert_eq!((written, method), (real.clone(), SaveMethod::Atomic));
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&real).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn hard_links_are_written_in_place() {
        let dir = TempDir::new("hard");
        let a = dir.0.join("a.md");
        let b = dir.0.join("b.md");
        fs::write(&a, "old").unwrap();
        fs::hard_link(&a, &b).unwrap();
        assert_eq!(save(&a, b"new").unwrap().1, SaveMethod::InPlace);
        assert_eq!(fs::read(&b).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn read_only_directory_falls_back_to_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("rodir");
        let path = dir.0.join("doc.md");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o555)).unwrap();
        let result = save(&path, b"new");
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o755)).unwrap();
        // Running as root ignores directory permissions; then the atomic path is fine too.
        assert!(result.is_ok());
        assert_eq!(fs::read(&path).unwrap(), b"new");
    }
}
