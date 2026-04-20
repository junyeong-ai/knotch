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
    pub fn register(&mut self, migrator: Box<dyn SchemaMigrator>) -> Result<(), MigrationError> {
        if self.migrators.iter().any(|m| m.from() == migrator.from()) {
            return Err(MigrationError::Overlap { from: migrator.from() });
        }
        self.migrators.push(migrator);
        Ok(())
    }

    /// Apply the registered chain of migrators to carry `value` from
    /// schema version `from` up to `to`. Each step advances by one
    /// version; schema downgrades are never supported because they
    /// can silently invalidate fingerprints.
    ///
    /// # Errors
    ///
    /// - [`MigrationError::Downgrade`] if `to < from`.
    /// - [`MigrationError::MissingLink`] if no registered migrator
    ///   bridges some intermediate version.
    /// - [`MigrationError::Custom`] if a migrator itself fails.
    pub fn migrate(&self, value: Value, from: u32, to: u32) -> Result<Value, MigrationError> {
        if from == to {
            return Ok(value);
        }
        if from > to {
            return Err(MigrationError::Downgrade { from, to });
        }
        let mut current = value;
        let mut version = from;
        while version < to {
            let step = self
                .migrators
                .iter()
                .find(|m| m.from() == version)
                .ok_or(MigrationError::MissingLink { from: version, to: version + 1 })?;
            current = step.migrate(current)?;
            version = step.to();
        }
        Ok(current)
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
    /// Downgrade attempted (`to < from`). Schema downgrades are not
    /// supported — they invalidate fingerprints for any event
    /// already canonicalized against the higher version.
    #[error("schema downgrade not supported: {from} → {to}")]
    Downgrade {
        /// Source version.
        from: u32,
        /// Target version (smaller than source).
        to: u32,
    },
    /// Implementation-defined migrator failure.
    #[error("migrator failed")]
    Custom(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Adds `added_field` to every object. Represents a typical
    /// "additive" schema step.
    struct AddFieldMigrator {
        from_version: u32,
        field_name: &'static str,
        field_value: Value,
    }

    impl SchemaMigrator for AddFieldMigrator {
        fn from(&self) -> u32 {
            self.from_version
        }
        fn migrate(&self, value: Value) -> Result<Value, MigrationError> {
            let Value::Object(mut map) = value else {
                return Ok(value);
            };
            map.insert(self.field_name.to_owned(), self.field_value.clone());
            Ok(Value::Object(map))
        }
    }

    /// Always errors — verifies `MigrationError::Custom` propagation.
    struct FailingMigrator;

    impl SchemaMigrator for FailingMigrator {
        fn from(&self) -> u32 {
            9
        }
        fn migrate(&self, _value: Value) -> Result<Value, MigrationError> {
            Err(MigrationError::Custom(Box::new(std::io::Error::other(
                "simulated migrator failure",
            ))))
        }
    }

    #[test]
    fn single_step_migration_rewrites_value() {
        let mut registry = Registry::new();
        registry
            .register(Box::new(AddFieldMigrator {
                from_version: 1,
                field_name: "source",
                field_value: json!("agent"),
            }))
            .expect("register");

        let migrated = registry.migrate(json!({ "kind": "unit_created" }), 1, 2).expect("migrate");
        assert_eq!(migrated, json!({ "kind": "unit_created", "source": "agent" }));
    }

    #[test]
    fn chain_migration_composes_each_step() {
        let mut registry = Registry::new();
        registry
            .register(Box::new(AddFieldMigrator {
                from_version: 1,
                field_name: "v2_field",
                field_value: json!(true),
            }))
            .unwrap();
        registry
            .register(Box::new(AddFieldMigrator {
                from_version: 2,
                field_name: "v3_field",
                field_value: json!(42),
            }))
            .unwrap();

        let migrated = registry.migrate(json!({ "base": "start" }), 1, 3).expect("migrate 1→3");
        assert_eq!(migrated, json!({ "base": "start", "v2_field": true, "v3_field": 42 }));
    }

    #[test]
    fn identity_migration_when_from_equals_to() {
        let registry = Registry::new();
        let original = json!({ "unchanged": true });
        let migrated = registry.migrate(original.clone(), 3, 3).expect("migrate");
        assert_eq!(migrated, original);
    }

    #[test]
    fn downgrade_is_rejected() {
        let registry = Registry::new();
        let err = registry.migrate(json!({}), 3, 1).expect_err("reject downgrade");
        assert!(matches!(err, MigrationError::Downgrade { from: 3, to: 1 }));
    }

    #[test]
    fn missing_link_is_reported_with_precise_version() {
        let mut registry = Registry::new();
        registry
            .register(Box::new(AddFieldMigrator {
                from_version: 1,
                field_name: "v2_field",
                field_value: json!(true),
            }))
            .unwrap();
        // Asking for 1→3 but only 1→2 is registered — should fail at
        // step 2→3.
        let err = registry.migrate(json!({}), 1, 3).expect_err("missing 2→3");
        assert!(matches!(err, MigrationError::MissingLink { from: 2, to: 3 }));
    }

    #[test]
    fn duplicate_registration_is_rejected() {
        let mut registry = Registry::new();
        registry
            .register(Box::new(AddFieldMigrator {
                from_version: 1,
                field_name: "x",
                field_value: json!(1),
            }))
            .unwrap();
        let err = registry
            .register(Box::new(AddFieldMigrator {
                from_version: 1,
                field_name: "y",
                field_value: json!(2),
            }))
            .expect_err("reject duplicate");
        assert!(matches!(err, MigrationError::Overlap { from: 1 }));
    }

    #[test]
    fn migrator_failure_surfaces_as_custom_error() {
        let mut registry = Registry::new();
        registry.register(Box::new(FailingMigrator)).unwrap();
        let err = registry.migrate(json!({}), 9, 10).expect_err("migrator returned Err");
        assert!(matches!(err, MigrationError::Custom(_)));
    }
}
