//! File-system-backed `Storage` adapter.
//!
//! Layout:
//!
//! ```text
//! <root>/
//!   <unit-id>/
//!     log.jsonl            # header + one event per line
//!     .resume-cache.json   # per-machine watermark (gitignored by convention)
//!     .lock                # advisory file lock (knotch-lock)
//!     .lock.meta           # lock-holder metadata (knotch-lock)
//! ```
//!
//! The adapter knows nothing about event shape — only bytes. Kernel
//! code is responsible for serializing `Event<W>` into JSONL lines
//! and interpreting the header sentinel.

use std::{
    path::{Path, PathBuf},
    pin::Pin,
};

use futures::{Stream, StreamExt as _, stream};
use knotch_kernel::UnitId;
use tokio::io::{AsyncReadExt as _, BufReader};

use crate::{
    Storage, atomic,
    error::StorageError,
    load_report::{CorruptionSpan, LoadReport},
};

/// Default file-system storage adapter.
#[derive(Debug, Clone)]
pub struct FileSystemStorage {
    root: PathBuf,
}

impl FileSystemStorage {
    /// Construct a storage rooted at `root`. The directory is created
    /// on demand; callers need not pre-create it.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Return the root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory that holds a unit's on-disk artefacts (`log.jsonl`,
    /// `.resume-cache.json`, `.lock`, `.lock.meta`).
    #[must_use]
    pub fn unit_dir(&self, unit: &UnitId) -> PathBuf {
        self.root.join(unit.as_str())
    }

    /// Absolute path to a unit's event log. This is the single
    /// authority for the `log.jsonl` file location — callers outside
    /// this crate (CLI, doctor, migrate, tests) go through this
    /// method rather than constructing the path themselves, so the
    /// filesystem layout stays a storage-adapter concern and
    /// `knotch-linter` R1 has nothing to flag in those crates.
    #[must_use]
    pub fn log_path(&self, unit: &UnitId) -> PathBuf {
        self.unit_dir(unit).join("log.jsonl")
    }

    /// Absolute path to a unit's resume-cache file. Non-authoritative
    /// projection over the log, gitignored by convention.
    #[must_use]
    pub fn cache_path(&self, unit: &UnitId) -> PathBuf {
        self.unit_dir(unit).join(".resume-cache.json")
    }
}

impl Storage for FileSystemStorage {
    async fn load(&self, unit: &UnitId) -> Result<(Vec<String>, LoadReport), StorageError> {
        let path = self.log_path(unit);
        match tokio::fs::File::open(&path).await {
            Ok(file) => {
                let mut buf = String::new();
                let mut reader = BufReader::new(file);
                let bytes = reader
                    .read_to_string(&mut buf)
                    .await
                    .map_err(|e| StorageError::io(path.clone(), e))?;
                let (lines, corrupted) = split_lines(&buf);
                Ok((lines, LoadReport { corrupted, bytes_read: bytes as u64 }))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Ok((Vec::new(), LoadReport::default()))
            }
            Err(err) => Err(StorageError::io(path, err)),
        }
    }

    async fn append(
        &self,
        unit: &UnitId,
        expected_len: u64,
        lines: Vec<String>,
    ) -> Result<(), StorageError> {
        let path = self.log_path(unit);
        let existing = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(StorageError::io(path.clone(), err)),
        };

        let on_disk_len = count_lines(&existing);
        if on_disk_len != expected_len {
            return Err(StorageError::LogMutated { expected: expected_len, on_disk: on_disk_len });
        }

        let mut combined = existing;
        if !combined.is_empty() && !combined.ends_with(b"\n") {
            combined.push(b'\n');
        }
        for line in &lines {
            combined.extend_from_slice(line.as_bytes());
            combined.push(b'\n');
        }

        atomic::write(&path, &combined).await.map_err(|e| StorageError::io(path, e))?;
        Ok(())
    }

    fn list_units(
        &self,
    ) -> Pin<Box<dyn Stream<Item = Result<UnitId, StorageError>> + Send + 'static>> {
        let root = self.root.clone();
        Box::pin(stream::once(async move { collect_units(root).await }).flat_map(stream::iter))
    }

    async fn read_cache(
        &self,
        unit: &UnitId,
    ) -> Result<serde_json::Map<String, serde_json::Value>, StorageError> {
        let path = self.cache_path(unit);
        match tokio::fs::read(&path).await {
            Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(serde_json::Value::Object(map)) => Ok(map),
                Ok(_) | Err(_) => Ok(serde_json::Map::new()),
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::Map::new()),
            Err(err) => Err(StorageError::io(path, err)),
        }
    }

    async fn write_cache(
        &self,
        unit: &UnitId,
        cache: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), StorageError> {
        let path = self.cache_path(unit);
        let bytes = serde_json::to_vec(&serde_json::Value::Object(cache)).map_err(|e| {
            StorageError::io_bare(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })?;
        atomic::write(&path, &bytes).await.map_err(|e| StorageError::io(path, e))?;
        Ok(())
    }
}

async fn collect_units(root: PathBuf) -> Vec<Result<UnitId, StorageError>> {
    let mut entries = match tokio::fs::read_dir(&root).await {
        Ok(e) => e,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => return vec![Err(StorageError::io(root, err))],
    };
    let mut out = Vec::new();
    loop {
        match entries.next_entry().await {
            Ok(Some(entry)) => {
                let ty = match entry.file_type().await {
                    Ok(t) => t,
                    Err(err) => {
                        out.push(Err(StorageError::io(entry.path(), err)));
                        continue;
                    }
                };
                if ty.is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        let candidate = entry.path().join("log.jsonl");
                        if tokio::fs::metadata(&candidate).await.is_ok() {
                            out.push(Ok(UnitId::new(name)));
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(err) => {
                out.push(Err(StorageError::io(root.clone(), err)));
                break;
            }
        }
    }
    out
}

fn count_lines(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count() as u64;
    if !bytes.ends_with(b"\n") { newlines + 1 } else { newlines }
}

fn split_lines(content: &str) -> (Vec<String>, Vec<CorruptionSpan>) {
    // Phase 2 treats any non-empty line as a candidate; corruption
    // detection moves to the Repository layer when it parses into
    // `Event<W>`. We preserve the line shape here so the Repository
    // can emit CorruptionSpan with accurate 1-indexed line numbers.
    let mut lines = Vec::new();
    for raw in content.split_inclusive('\n') {
        let trimmed = raw.strip_suffix('\n').unwrap_or(raw);
        if !trimmed.is_empty() {
            lines.push(trimmed.to_owned());
        }
    }
    (lines, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage() -> (tempfile::TempDir, FileSystemStorage) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = FileSystemStorage::new(dir.path());
        (dir, storage)
    }

    #[tokio::test]
    async fn load_of_missing_unit_is_empty() {
        let (_dir, storage) = storage();
        let unit = UnitId::new("nope");
        let (lines, report) = storage.load(&unit).await.expect("load");
        assert!(lines.is_empty());
        assert!(report.is_clean());
    }

    #[tokio::test]
    async fn append_then_load_roundtrips() {
        let (_dir, storage) = storage();
        let unit = UnitId::new("signup");
        storage
            .append(&unit, 0, vec!["{\"a\":1}".into(), "{\"a\":2}".into()])
            .await
            .expect("append");
        let (lines, report) = storage.load(&unit).await.expect("load");
        assert_eq!(lines, vec!["{\"a\":1}".to_string(), "{\"a\":2}".to_string()]);
        assert!(report.is_clean());
        assert!(report.bytes_read > 0);
    }

    #[tokio::test]
    async fn append_with_wrong_expected_len_errors() {
        let (_dir, storage) = storage();
        let unit = UnitId::new("race");
        storage.append(&unit, 0, vec!["{\"a\":1}".into()]).await.expect("first");
        let err = storage
            .append(&unit, 0, vec!["{\"a\":2}".into()]) // stale expected_len
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::LogMutated { on_disk: 1, expected: 0 }));
    }

    #[tokio::test]
    async fn cache_roundtrips() {
        let (_dir, storage) = storage();
        let unit = UnitId::new("cache-unit");
        let mut map = serde_json::Map::new();
        map.insert("head".into(), serde_json::Value::String("abc".into()));
        storage.write_cache(&unit, map.clone()).await.expect("write");
        let read = storage.read_cache(&unit).await.expect("read");
        assert_eq!(read, map);
    }

    #[tokio::test]
    async fn missing_cache_returns_empty_map() {
        let (_dir, storage) = storage();
        let map = storage.read_cache(&UnitId::new("nope")).await.expect("read");
        assert!(map.is_empty());
    }

    #[test]
    fn count_lines_handles_trailing_newline_absence() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a\n"), 1);
        assert_eq!(count_lines(b"a\nb\n"), 2);
        assert_eq!(count_lines(b"a\nb"), 2);
    }
}
