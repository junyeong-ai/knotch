//! Shared test fixtures, in-memory adapters, and simulation harness.
//!
//! Intentionally separated from production crates so `InMemoryVcs`
//! and similar fakes cannot accidentally ship. Consumers pull this
//! crate in as a `dev-dependency` only.

pub mod repo;
pub mod sim;
pub mod vcs;

pub use self::{
    repo::InMemoryRepository,
    vcs::{InMemoryVcs, VcsFixture},
};
