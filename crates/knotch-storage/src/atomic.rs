//! Atomic file-write primitives.
//!
//! The writer lays down a temporary file, fsyncs it, then renames
//! over the target. On POSIX `rename(2)` is atomic; on Windows
//! `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` is atomic as of
//! Windows 10 1709. Retry on `ERROR_SHARING_VIOLATION` handles
//! anti-virus / backup-agent contention.

use std::{
    io,
    path::{Path, PathBuf},
};

use tokio::io::AsyncWriteExt;

const SHARING_VIOLATION_RETRIES: u32 = 8;

/// Atomically replace `target` with the supplied bytes.
///
/// Flow: pick `target`-with-random-suffix as temp path, write,
/// fsync the temp file and its parent directory, then rename.
///
/// # Errors
/// Any underlying I/O failure surfaces as `io::Error`.
pub async fn write(target: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = target.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "target has no parent directory")
    })?;
    tokio::fs::create_dir_all(parent).await?;

    let tmp = temp_path_for(target);
    {
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(contents).await?;
        file.flush().await?;
        file.sync_all().await?;
    }

    rename_with_retry(&tmp, target).await?;
    fsync_dir(parent).await?;
    Ok(())
}

async fn rename_with_retry(src: &Path, dst: &Path) -> io::Result<()> {
    for attempt in 0..SHARING_VIOLATION_RETRIES {
        match tokio::fs::rename(src, dst).await {
            Ok(()) => return Ok(()),
            Err(err) if is_sharing_violation(&err) && attempt + 1 < SHARING_VIOLATION_RETRIES => {
                let delay_ms = 16_u64 * (1_u64 << attempt);
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            Err(err) => return Err(err),
        }
    }
    tokio::fs::rename(src, dst).await
}

#[cfg(windows)]
fn is_sharing_violation(err: &io::Error) -> bool {
    // ERROR_SHARING_VIOLATION (32)
    err.raw_os_error() == Some(32)
}

#[cfg(not(windows))]
fn is_sharing_violation(_: &io::Error) -> bool {
    false
}

#[cfg(unix)]
async fn fsync_dir(dir: &Path) -> io::Result<()> {
    let dir = dir.to_owned();
    tokio::task::spawn_blocking(move || -> io::Result<()> {
        let f = std::fs::File::open(&dir)?;
        f.sync_all()
    })
    .await
    .map_err(io::Error::other)?
}

#[cfg(not(unix))]
async fn fsync_dir(_dir: &Path) -> io::Result<()> {
    // Windows does not support directory fsync; the rename itself is
    // durable once its metadata hits disk.
    Ok(())
}

fn temp_path_for(target: &Path) -> PathBuf {
    let mut bytes = [0u8; 8];
    getrandom_fallback(&mut bytes);
    let nonce = u64::from_le_bytes(bytes);
    let suffix = format!(".knotch-tmp-{nonce:016x}");
    let mut buf = target.as_os_str().to_os_string();
    buf.push(&suffix);
    PathBuf::from(buf)
}

/// Cheap nonce fallback — used only for temp-file suffixes so
/// collisions are harmless. Intentionally avoids pulling `rand`.
fn getrandom_fallback(buf: &mut [u8]) {
    use std::{
        hash::{BuildHasher, Hasher},
        time::{SystemTime, UNIX_EPOCH},
    };

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0);
    let hasher_state = std::collections::hash_map::RandomState::new();
    let mut hasher = hasher_state.build_hasher();
    hasher.write_u64(nanos);
    hasher.write_u64(std::process::id() as u64);
    let seed = hasher.finish();

    for (i, slot) in buf.iter_mut().enumerate() {
        let shift = (i % 8) * 8;
        *slot = ((seed >> shift) & 0xFF) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_and_fsyncs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("log.jsonl");
        write(&target, b"hello\n").await.expect("write");
        let read = tokio::fs::read(&target).await.expect("read");
        assert_eq!(read, b"hello\n");
    }

    #[tokio::test]
    async fn overwrites_existing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("log.jsonl");
        write(&target, b"v1").await.expect("write v1");
        write(&target, b"v2-longer").await.expect("write v2");
        let read = tokio::fs::read(&target).await.expect("read");
        assert_eq!(read, b"v2-longer");
    }

    #[tokio::test]
    async fn creates_parent_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("nested/a/b/c/log.jsonl");
        write(&target, b"ok").await.expect("write");
        assert!(target.exists());
    }

    #[test]
    fn temp_path_appends_suffix() {
        let p = temp_path_for(Path::new("/tmp/log.jsonl"));
        let s = p.to_string_lossy();
        assert!(s.starts_with("/tmp/log.jsonl.knotch-tmp-"));
    }
}
