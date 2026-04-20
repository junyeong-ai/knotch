//! Tracing attribute schema + span-construction helpers for knotch.
//!
//! The attribute keys exposed here are part of the stable public
//! surface — `cargo-public-api` diffs would catch a drift. Consumers
//! depending on these strings (dashboards, alert rules) can pin to
//! a semver-tracked version.

pub mod attrs;
pub mod spans;

pub use self::{attrs::Attrs, spans::emit_append, spans::emit_reconcile};

/// Convenience facade: install the standard tracing subscriber with
/// knotch's attribute formatter. Returns the subscriber's guard; drop
/// when shutting down.
///
/// # Errors
/// Returns the subscriber's `TryInitError` when a global subscriber
/// is already installed.
pub fn install_default()
-> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().with_target(true))
        .try_init()?;
    Ok(())
}
