//! Atomic, crash-safe file write.
//!
//! Every `.knotch/` writer (active pointers, reconciler queue,
//! subagent records) routes through [`write`]. The guarantee:
//!
//! > After [`write`] returns `Ok(())`, either the prior file content
//! > or the full new content is on disk — never a partial write,
//! > even across a power loss / kernel panic.
//!
//! ## Sequence
//!
//! 1. Create `<path>.tmp`, write every byte, `fsync` the file.
//! 2. `rename(<path>.tmp, <path>)` — POSIX-atomic, but the dirent
//!    change still lives in the kernel's pending-writes queue.
//! 3. Open the parent directory and `fsync` it. This commits the
//!    dirent change, so after a crash the rename is durable.
//!
//! The parent-fsync step is why bare `std::fs::write` + `rename` is
//! **not** crash-safe despite rename being "atomic" — atomicity is
//! about kernel visibility, not disk persistence. SQLite / LMDB /
//! PostgreSQL all do the same thing.
//!
//! ## Platform notes
//!
//! - **Unix**: `File::open(parent).sync_all()` is the standard
//!   durability barrier.
//! - **Windows**: directory handles do not support `FlushFileBuffers`
//!   the same way; NTFS flushes dirent changes eagerly enough that
//!   the file sync is typically sufficient. The parent fsync is
//!   gated behind `cfg(unix)` for now.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Atomically write `bytes` to `path`. See module docs for the
/// durability contract.
///
/// # Errors
/// Any `std::io::Error` from create / write / fsync / rename.
pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    let tmp = temp_path(path)?;

    // 1. Write + fsync the new content.
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }

    // 2. POSIX-atomic rename.
    std::fs::rename(&tmp, path)?;

    // 3. Commit the dirent change so the rename survives a crash.
    sync_parent_dir(parent)?;

    Ok(())
}

fn temp_path(path: &Path) -> std::io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    let mut tmp_name = name.to_os_string();
    tmp_name.push(".tmp");
    Ok(path.with_file_name(tmp_name))
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> std::io::Result<()> {
    let dir = std::fs::File::open(parent)?;
    dir.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> std::io::Result<()> {
    // Windows flushes dirent changes as part of the rename FS
    // operation; explicit directory fsync is not required here.
    // If ReFS / SMB scenarios start showing drift, revisit with
    // platform-specific handle-based flushing.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_new_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.toml");
        write(&path, b"first").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
    }

    #[test]
    fn atomic_write_overwrites_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.toml");
        std::fs::write(&path, b"initial").unwrap();
        write(&path, b"replaced").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "replaced");
    }

    #[test]
    fn atomic_write_leaves_no_tmp_on_success() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.toml");
        write(&path, b"x").unwrap();
        assert!(!path.with_extension("toml.tmp").exists());
    }

    #[test]
    fn atomic_write_fails_when_parent_missing() {
        let path = Path::new("/definitely/does/not/exist/file.toml");
        assert!(write(path, b"x").is_err());
    }
}
