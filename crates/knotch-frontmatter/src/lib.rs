//! Keep a Markdown file's YAML frontmatter in sync with a knotch
//! unit's ledger status.
//!
//! The event log is authoritative; frontmatter is a projection of
//! the log into a human-readable header block (constitution §I).
//! When a unit's status transitions, adopters that keep a
//! Markdown file per unit use this crate to rewrite the header so
//! operators see the same status in the file as in the ledger.
//!
//! The primitives below are deliberately minimal: parse + emit +
//! atomic file sync. The crate does **not** drive log events —
//! adopters call [`sync_status_on_file`] from whatever hook /
//! skill fires when their projection detects a status change.
//!
//! ```no_run
//! # async fn run() -> Result<(), knotch_frontmatter::FrontmatterError> {
//! use knotch_frontmatter::sync_status_on_file;
//! sync_status_on_file("specs/spring-sale/spec.md", "shipped").await?;
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::{io, path::Path};

use compact_str::CompactString;
pub use knotch_schema::{FieldSchema, FieldType, FrontmatterSchema, SchemaError};
use serde_json::{Map, Value};
use yaml_serde::Value as YamlValue;

/// Errors produced by the frontmatter utilities.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FrontmatterError {
    /// I/O failure on the Markdown file.
    #[error("io: {source}")]
    Io {
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// The file has no YAML frontmatter block.
    #[error("no frontmatter block — expected `---` fence at the top of the file")]
    NoFrontmatter,
    /// The YAML frontmatter is malformed.
    #[error("yaml parse: {source}")]
    Yaml {
        /// Underlying YAML parse error.
        #[source]
        source: yaml_serde::Error,
    },
    /// The frontmatter parses but its top-level isn't a mapping.
    #[error("frontmatter is not a mapping — expected a YAML object at the top")]
    NotAnObject,
    /// Schema validation rejected the frontmatter object.
    #[error("schema: {source}")]
    Schema {
        /// Underlying schema violation.
        #[source]
        source: SchemaError,
    },
}

impl From<io::Error> for FrontmatterError {
    fn from(source: io::Error) -> Self {
        Self::Io { source }
    }
}

impl From<yaml_serde::Error> for FrontmatterError {
    fn from(source: yaml_serde::Error) -> Self {
        Self::Yaml { source }
    }
}

impl From<SchemaError> for FrontmatterError {
    fn from(source: SchemaError) -> Self {
        Self::Schema { source }
    }
}

/// Parsed Markdown document — separated frontmatter header + body.
#[derive(Debug, Clone)]
pub struct Document {
    header: Map<String, Value>,
    body: String,
}

impl Document {
    /// Parse a Markdown string. Requires a YAML frontmatter block
    /// fenced by `---\n` on the very first line and closed by a
    /// matching `---` on its own line.
    ///
    /// # Errors
    /// Returns [`FrontmatterError::NoFrontmatter`] when no fence is
    /// found, [`FrontmatterError::Yaml`] on parse failure, and
    /// [`FrontmatterError::NotAnObject`] when the YAML isn't a
    /// mapping.
    pub fn parse(markdown: &str) -> Result<Self, FrontmatterError> {
        let (front, body) = split_frontmatter(markdown).ok_or(FrontmatterError::NoFrontmatter)?;
        let yaml: YamlValue = yaml_serde::from_str(front)?;
        let YamlValue::Mapping(map) = yaml else {
            return Err(FrontmatterError::NotAnObject);
        };
        let json = yaml_map_to_json(map)?;
        Ok(Self { header: json, body: body.to_owned() })
    }

    /// Look up a field in the frontmatter.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.header.get(key)
    }

    /// Insert or update a field. Returns the previous value if any.
    pub fn set(&mut self, key: impl Into<String>, value: Value) -> Option<Value> {
        self.header.insert(key.into(), value)
    }

    /// Remove a field. Returns the previous value if any.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.header.remove(key)
    }

    /// Read-only view of every frontmatter field.
    #[must_use]
    pub fn header(&self) -> &Map<String, Value> {
        &self.header
    }

    /// Body text after the frontmatter.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Validate the header against a [`FrontmatterSchema`].
    ///
    /// # Errors
    /// Returns [`FrontmatterError::Schema`] on any violation.
    pub fn validate(&self, schema: &FrontmatterSchema) -> Result<(), FrontmatterError> {
        schema.validate(&self.header).map_err(Into::into)
    }

    /// Render the document back to a Markdown string with an
    /// updated frontmatter block. Preserves the body verbatim.
    ///
    /// # Errors
    /// Returns [`FrontmatterError::Yaml`] if the header cannot be
    /// serialised back to YAML (rare — only happens for exotic
    /// `serde_json::Value` shapes).
    pub fn to_markdown(&self) -> Result<String, FrontmatterError> {
        let yaml = yaml_serde::to_string(&self.header)?;
        Ok(format!("---\n{yaml}---\n{body}", body = self.body))
    }
}

/// Atomically rewrite the frontmatter `status` field of a Markdown
/// file to match `new_status`. No-op when the header already carries
/// that value.
///
/// Uses a temp-file + rename pattern inside the same directory for
/// crash safety. The body of the Markdown file is preserved
/// verbatim.
///
/// # Errors
/// Returns [`FrontmatterError`] for parse / I/O / serialisation
/// failures.
pub async fn sync_status_on_file(
    path: impl AsRef<Path>,
    new_status: &str,
) -> Result<(), FrontmatterError> {
    let path = path.as_ref();
    let raw = tokio::fs::read_to_string(path).await?;
    let mut doc = Document::parse(&raw)?;
    if matches!(doc.get("status"), Some(Value::String(s)) if s == new_status) {
        return Ok(());
    }
    doc.set("status", Value::String(new_status.to_owned()));
    let rendered = doc.to_markdown()?;
    atomic_write(path, rendered.as_bytes()).await
}

/// Low-level atomic write: emit to a sibling temp file, fsync, then
/// rename over the destination. Pulled out so adopters can reuse
/// the write path when they hand-construct a [`Document`].
///
/// # Errors
/// Returns [`FrontmatterError::Io`] on filesystem failure.
pub async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), FrontmatterError> {
    use tokio::io::AsyncWriteExt as _;

    let parent =
        path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut tmp = parent.join(file_name);
    let mut tmp_name = tmp.file_name().unwrap().to_os_string();
    tmp_name.push(".knotch-tmp");
    tmp.set_file_name(tmp_name);

    {
        let mut f = tokio::fs::File::create(&tmp).await?;
        f.write_all(bytes).await?;
        f.sync_all().await?;
    }
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

fn split_frontmatter(markdown: &str) -> Option<(&str, &str)> {
    let rest = markdown.strip_prefix("---\n")?;
    let end = rest.find("\n---\n").or_else(|| rest.find("\n---"))?;
    let front = &rest[..end];
    let after =
        rest[end..].strip_prefix("\n---\n").or_else(|| rest[end..].strip_prefix("\n---"))?;
    Some((front, after))
}

fn yaml_map_to_json(map: yaml_serde::Mapping) -> Result<Map<String, Value>, FrontmatterError> {
    let mut out = Map::with_capacity(map.len());
    for (k, v) in map {
        let YamlValue::String(key) = k else {
            return Err(FrontmatterError::NotAnObject);
        };
        out.insert(key, yaml_to_json(v)?);
    }
    Ok(out)
}

fn yaml_to_json(yaml: YamlValue) -> Result<Value, FrontmatterError> {
    Ok(match yaml {
        YamlValue::Null => Value::Null,
        YamlValue::Bool(b) => Value::Bool(b),
        YamlValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                Value::Number(u.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        YamlValue::String(s) => Value::String(s),
        YamlValue::Sequence(seq) => {
            Value::Array(seq.into_iter().map(yaml_to_json).collect::<Result<_, _>>()?)
        }
        YamlValue::Mapping(map) => {
            let json = yaml_map_to_json(map)?;
            Value::Object(json)
        }
        YamlValue::Tagged(boxed) => yaml_to_json(boxed.value)?,
    })
}

/// Re-exported for convenience so adopters don't need to pull in
/// `compact_str` directly when building a [`FrontmatterSchema`].
pub use compact_str as __compact_str;
#[doc(hidden)]
pub type __FieldName = CompactString;

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\n\
id: spec-001\n\
title: Test Spec\n\
status: in_progress\n\
---\n\
# Body\n\
Some content.\n";

    #[test]
    fn parse_extracts_header_and_body() {
        let doc = Document::parse(SAMPLE).expect("parse");
        assert_eq!(doc.get("id").and_then(Value::as_str), Some("spec-001"));
        assert_eq!(doc.get("status").and_then(Value::as_str), Some("in_progress"));
        assert!(doc.body().starts_with("# Body"));
    }

    #[test]
    fn set_updates_field_value() {
        let mut doc = Document::parse(SAMPLE).expect("parse");
        doc.set("status", Value::String("shipped".into()));
        assert_eq!(doc.get("status").and_then(Value::as_str), Some("shipped"));
    }

    #[test]
    fn to_markdown_round_trips_body() {
        let mut doc = Document::parse(SAMPLE).expect("parse");
        doc.set("status", Value::String("shipped".into()));
        let rendered = doc.to_markdown().expect("emit");
        assert!(rendered.contains("status: shipped"));
        assert!(rendered.contains("# Body"));
        assert!(rendered.contains("Some content."));
    }

    #[test]
    fn parse_rejects_missing_fence() {
        let err = Document::parse("# no frontmatter\nbody").unwrap_err();
        assert!(matches!(err, FrontmatterError::NoFrontmatter));
    }

    #[test]
    fn validate_consults_schema() {
        let schema = FrontmatterSchema::builder()
            .field(FieldSchema::required("id", FieldType::String))
            .field(FieldSchema::required("title", FieldType::String))
            .field(FieldSchema::required("status", FieldType::String));
        let doc = Document::parse(SAMPLE).expect("parse");
        doc.validate(&schema).expect("valid frontmatter");
    }

    #[tokio::test]
    async fn sync_status_on_file_is_idempotent_no_op_when_already_matching() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("spec.md");
        tokio::fs::write(&path, SAMPLE).await.expect("write");
        sync_status_on_file(&path, "in_progress").await.expect("sync");
        let raw = tokio::fs::read_to_string(&path).await.expect("read");
        // Idempotent — same content (we chose not to rewrite when the
        // value already matches).
        assert_eq!(raw, SAMPLE);
    }

    #[tokio::test]
    async fn sync_status_on_file_rewrites_when_status_differs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("spec.md");
        tokio::fs::write(&path, SAMPLE).await.expect("write");
        sync_status_on_file(&path, "shipped").await.expect("sync");
        let raw = tokio::fs::read_to_string(&path).await.expect("read");
        assert!(raw.contains("status: shipped"));
        assert!(raw.contains("# Body"));
    }
}
