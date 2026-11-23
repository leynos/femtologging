//! Compile-time UI tests for PyO3 patterns used by the maturin build.
//!
//! `compile_time_ui` runs `trybuild` `TestCases` against this crate's
//! public surface: the `tests/ui/pass/*.rs` cases must build, including
//! `handler_flush_until.rs` and `queued_record_context.rs`, while
//! `tests/ui/fail/*.rs` must produce the expected diagnostics. Cases gated
//! by features, such as `log_bridge_reexports.rs` and
//! `handle_expect_wrappers.rs`, reflect which re-exports are visible in
//! each build.

#[test]
fn compile_time_ui() {
    let test_cases = trybuild::TestCases::new();
    test_cases.pass("tests/ui/pass/queued_record_context.rs");
    test_cases.pass("tests/ui/pass/file_test_support.rs");
    test_cases.pass("tests/ui/pass/handler_formatter_builder.rs");
    test_cases.pass("tests/ui/pass/immutable_handler_close.rs");
    test_cases.pass("tests/ui/pass/femto_logger_fallible_construction.rs");
    test_cases.pass("tests/ui/pass/handler_flush_until.rs");
    test_cases.pass("tests/ui/pass/production_api.rs");
    // The bridge re-exports exist only where the feature gating exposes them.
    #[cfg(all(feature = "log-compat", feature = "test-util"))]
    test_cases.pass("tests/ui/pass/log_bridge_reexports.rs");
    // The wrapper case needs a Python-linked rlib, which extension-module
    // builds intentionally omit. It runs in the no-extension feature lanes.
    #[cfg(not(feature = "extension-module"))]
    test_cases.pass("tests/ui/pass/handle_expect_wrappers.rs");
    test_cases.pass("tests/ui/pass/pyo3_pymodule.rs");
    test_cases.pass("tests/ui/pass/pyo3_signature.rs");
    test_cases.pass("tests/ui/pass/pyo3_detach_ungil.rs");
    test_cases.pass("tests/ui/pass/schema_version_error_wildcard.rs");
    test_cases.compile_fail("tests/ui/fail/*.rs");
}
