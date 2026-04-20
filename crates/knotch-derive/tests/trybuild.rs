//! UI-error corpus for the `#[workflow]` attribute macro and the
//! derive macros. Run with `cargo test -p knotch-derive --test
//! trybuild`; snapshot files live in `tests/ui/`.
//!
//! Running this is gated by `TRYBUILD=overwrite` to regenerate
//! expected output.

#[test]
fn ui_fixtures() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/workflow_ok.rs");
    t.compile_fail("tests/ui/workflow_missing_name.rs");
    t.compile_fail("tests/ui/workflow_unknown_arg.rs");
    t.compile_fail("tests/ui/phase_kind_non_unit.rs");
}
