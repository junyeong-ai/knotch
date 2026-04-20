//! Schema migration registry.
//!
//! Knotch refuses to silently upgrade wire formats; every schema change is an
//! explicit user-supplied migrator. Migrators operate on `serde_json::Value`
//! → `serde_json::Value` so they do not require the historical `Event` enum.

use serde_json::Value;

/// One step of a migration chain from `from` → `to = from + 1`.
pub trait SchemaMigrator: Send + Sync {
    /// Source schema version this migrator upgrades from.
    fn from(&self) -> u32;
    /// Target schema version this migrator produces.
    fn to(&self) -> u32 {
        self.from() + 1
    }
    /// Apply the migration to a single event (or header) value.
    ///
    /// # Errors
    /// Implementation-defined; propagate via `MigrationError::Custom`.
    fn migrate(&self, value: Value) -> Result<Value, MigrationError>;
}

/// Registered chain of migrators. Lookup by `from` version.
#[derive(Default)]
pub struct Registry {
    migrators: Vec<Box<dyn SchemaMigrator>>,
}

impl Registry {
    /// Create an empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self { migrators: Vec::new() }
    }

    /// Register a migrator. Later steps in the chain must be registered in
    /// version order; duplicates are rejected.
    ///
    /// # Errors
    /// Returns [`MigrationError::Overlap`] if a migrator with the same
    /// `from` is already registered.
    pub fn register(
        &mut self,
        migrator: Box<dyn SchemaMigrator>,
    ) -> Result<(), MigrationError> {
        if self.migrators.iter().any(|m| m.from() == migrator.from()) {
            return Err(MigrationError::Overlap { from: migrator.from() });
        }
        self.migrators.push(migrator);
        Ok(())
    }
}

/// Migration-pipeline error taxonomy.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MigrationError {
    /// Two migrators claim the same `from` version.
    #[error("duplicate migrator registered for schema version {from}")]
    Overlap {
        /// The conflicting source version.
        from: u32,
    },
    /// No migrator found that can bridge the requested version.
    #[error("no migrator chain from {from} to {to}")]
    MissingLink {
        /// Starting schema version.
        from: u32,
        /// Target schema version.
        to: u32,
    },
    /// Implementation-defined migrator failure.
    #[error("migrator failed")]
    Custom(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}
