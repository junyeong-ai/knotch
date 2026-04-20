//! Workspace configuration.
//!
//! Resolution order (highest → lowest priority):
//!
//! 1. CLI `--root` flag (or `KNOTCH_ROOT` env).
//! 2. Nearest ancestor `knotch.toml` from cwd.
//! 3. Current working directory.
//!
//! Additional fields are layered via figment: file → env → defaults.

use std::path::{Path, PathBuf};

use figment::{Figment, providers::{Env, Format, Serialized, Toml}};
use knotch_observer::ObserverManifest;
use knotch_storage::FileRepository;
use knotch_workflow::ConfigWorkflow;
use serde::{Deserialize, Serialize};

/// Policy for the `guard-rewrite` hook.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GuardPolicy {
    /// Hard-block destructive git operations with exit 2.
    Block,
    /// Attach a warning to Claude's context but let the command run.
    /// The default — agent can see the risk and choose to proceed.
    #[default]
    Warn,
    /// Ignore destructive operations entirely. Escape hatch for
    /// experimental branches / solo projects.
    Off,
}

/// Guard-hook policy block. Currently only one knob; future
/// destructive classes (e.g. rm -rf of tracked dirs) can extend.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
pub(crate) struct GuardConfig {
    /// Policy for `git push --force` / `reset --hard` / `branch -D`
    /// / `checkout --` / `clean -f` / `rebase -i|--root`.
    #[serde(default)]
    pub rewrite: GuardPolicy,
}

/// Resolved CLI-run configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct Config {
    /// Workspace root — parent of the `state/` directory where each
    /// unit keeps its JSONL log.
    pub root: PathBuf,
    /// Directory under `root` that holds per-unit state.
    pub state_dir: PathBuf,
    /// Wire-schema version the workspace expects. Defaults to
    /// `knotch_proto::SCHEMA_VERSION`.
    pub schema_version: u32,
    /// Guard-hook policies.
    #[serde(default)]
    pub guard: GuardConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            state_dir: PathBuf::from("state"),
            schema_version: knotch_proto::SCHEMA_VERSION,
            guard: GuardConfig::default(),
        }
    }
}

impl Config {
    /// Build a `Config` using the CLI-supplied root override (if any)
    /// or by walking up from the current working directory.
    ///
    /// # Errors
    /// Returns a figment error when `knotch.toml` is present but
    /// malformed, or when the env-var overrides fail to parse.
    #[allow(clippy::result_large_err)]
    pub(crate) fn resolve(cli_root: Option<&Path>) -> figment::Result<Self> {
        let (root, toml_path) = discover_root(cli_root);
        let mut figment = Figment::from(Serialized::defaults(Config::default()));
        if let Some(path) = &toml_path {
            figment = figment.merge(Toml::file(path));
        }
        figment = figment.merge(Env::prefixed("KNOTCH_").split("__"));
        let mut cfg: Config = figment.extract()?;
        cfg.root = root;
        if cfg.state_dir.is_relative() {
            cfg.state_dir = cfg.root.join(&cfg.state_dir);
        }
        Ok(cfg)
    }

    /// Absolute path to a unit's directory.
    #[must_use]
    pub(crate) fn unit_dir(&self, unit: &str) -> PathBuf {
        self.state_dir.join(unit)
    }

    /// Absolute path to a unit's log file.
    #[must_use]
    pub(crate) fn unit_log(&self, unit: &str) -> PathBuf {
        self.unit_dir(unit).join("log.jsonl")
    }

    /// Path to the root `knotch.toml` (may not exist on disk yet).
    #[must_use]
    pub(crate) fn config_path(&self) -> PathBuf {
        self.root.join("knotch.toml")
    }

    /// Load the workflow declaration this project uses — the
    /// `[workflow]` table from `knotch.toml` if present, else the
    /// canonical shape.
    ///
    /// # Errors
    /// Surfaces `toml` parse errors + `ConfigWorkflow::load` validation.
    pub(crate) fn load_workflow(&self) -> anyhow::Result<ConfigWorkflow> {
        let toml_path = self.config_path();
        if !toml_path.exists() {
            return Ok(ConfigWorkflow::canonical());
        }
        let raw = std::fs::read_to_string(&toml_path)?;
        if !raw.contains("[workflow]") {
            return Ok(ConfigWorkflow::canonical());
        }
        Ok(ConfigWorkflow::load(&toml_path)?)
    }

    /// Build the file-backed repository this project uses. The
    /// workflow instance comes from [`Config::load_workflow`] and the
    /// storage root from [`Config::state_dir`].
    ///
    /// # Errors
    /// Propagates workflow load errors.
    pub(crate) fn build_repository(&self) -> anyhow::Result<FileRepository<ConfigWorkflow>> {
        let workflow = self.load_workflow()?;
        Ok(FileRepository::new(&self.state_dir, workflow))
    }

    /// Load the observer manifests declared in `knotch.toml`'s
    /// `[[observers]]` array. Returns an empty vec when the file is
    /// absent or has no observer declarations — the reconciler runs
    /// with only the first-party observers in that case.
    ///
    /// Consumed by `knotch doctor` to surface declared observers +
    /// missing binaries; also available to adopter-driven reconcile
    /// flows that wire declared observers into
    /// `knotch-reconciler::Reconciler` from their own binary (the
    /// shipped `knotch reconcile` subcommand only drains the queue).
    ///
    /// # Errors
    /// Propagates TOML parse errors; returns an empty list on
    /// missing file.
    pub(crate) fn load_observer_manifests(&self) -> anyhow::Result<Vec<ObserverManifest>> {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(default)]
            observers: Vec<ObserverManifest>,
        }

        let toml_path = self.config_path();
        if !toml_path.exists() {
            return Ok(Vec::new());
        }
        let raw = std::fs::read_to_string(&toml_path)?;
        let wrapper: Wrapper = toml::from_str(&raw)?;
        // Resolve each binary path against the project root so
        // observer declarations stay portable.
        let mut manifests = wrapper.observers;
        for m in &mut manifests {
            if m.binary.is_relative() {
                m.binary = self.root.join(&m.binary);
            }
        }
        Ok(manifests)
    }
}

fn discover_root(cli_root: Option<&Path>) -> (PathBuf, Option<PathBuf>) {
    if let Some(root) = cli_root {
        let toml = root.join("knotch.toml");
        let toml_opt = if toml.exists() { Some(toml) } else { None };
        return (root.to_path_buf(), toml_opt);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut cursor = cwd.as_path();
    loop {
        let candidate = cursor.join("knotch.toml");
        if candidate.exists() {
            return (cursor.to_path_buf(), Some(candidate));
        }
        match cursor.parent() {
            Some(parent) => cursor = parent,
            None => break,
        }
    }
    (cwd, None)
}
