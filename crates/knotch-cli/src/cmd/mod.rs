//! Subcommand entry points.

pub(crate) mod doctor;
pub(crate) mod gate;
pub(crate) mod hook;
pub(crate) mod init;
pub(crate) mod log;
pub(crate) mod mark;
pub(crate) mod migrate;
pub(crate) mod reconcile;
pub(crate) mod show;
pub(crate) mod supersede;
pub(crate) mod transition;
pub(crate) mod unit;

/// Output formatter switch honored by every subcommand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputMode {
    /// Human-readable summary.
    Human,
    /// Machine-parsable JSON on stdout.
    Json,
}

impl OutputMode {
    /// Is the mode JSON?
    #[must_use]
    pub(crate) const fn is_json(self) -> bool {
        matches!(self, Self::Json)
    }
}

/// Read all non-empty lines of a knotch JSONL log, tolerating a
/// missing file (returns an empty vec).
pub(crate) async fn read_log_lines(
    path: &std::path::Path,
) -> anyhow::Result<Vec<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(body) => Ok(body
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.is_empty())
            .map(ToOwned::to_owned)
            .collect()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(anyhow::Error::new(err).context(format!(
            "failed to read {}",
            path.display()
        ))),
    }
}
