//! `active.toml` read/write round-trip.

use knotch_agent::active::{ActiveUnit, resolve_active, write_active};
use knotch_kernel::UnitId;
use tempfile::TempDir;

#[test]
fn resolve_without_knotch_toml_is_no_project() {
    let tmp = TempDir::new().unwrap();
    let out = resolve_active(tmp.path()).unwrap();
    assert_eq!(out, ActiveUnit::NoProject);
}

#[test]
fn resolve_with_knotch_toml_but_no_active_is_uninitialized() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("knotch.toml"), "state_dir = \"state\"\n").unwrap();
    let out = resolve_active(tmp.path()).unwrap();
    assert_eq!(out, ActiveUnit::Uninitialized);
}

#[test]
fn write_then_resolve_roundtrips() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("knotch.toml"), "state_dir = \"state\"\n").unwrap();
    let unit = UnitId::try_new("signup-flow").unwrap();
    write_active(tmp.path(), Some(&unit), "test").unwrap();
    let out = resolve_active(tmp.path()).unwrap();
    assert_eq!(out, ActiveUnit::Active(unit));
}

#[test]
fn write_none_clears_active() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("knotch.toml"), "state_dir = \"state\"\n").unwrap();
    let unit = UnitId::try_new("signup-flow").unwrap();
    write_active(tmp.path(), Some(&unit), "test").unwrap();
    write_active(tmp.path(), None, "test").unwrap();
    assert_eq!(resolve_active(tmp.path()).unwrap(), ActiveUnit::Uninitialized);
}
