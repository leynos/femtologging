//! Compile-time UI tests for `PyO3` patterns used by the maturin build.

#[test]
fn compile_time_ui() {
    let test_cases = trybuild::TestCases::new();
    test_cases.pass("tests/ui/pass/file_test_support.rs");
    // The wrapper case needs a Python-linked rlib, which extension-module
    // builds intentionally omit. It runs in the no-extension feature lanes.
    #[cfg(not(feature = "extension-module"))]
    test_cases.pass("tests/ui/pass/handle_expect_wrappers.rs");
    test_cases.pass("tests/ui/pass/pyo3_pymodule.rs");
    test_cases.pass("tests/ui/pass/pyo3_signature.rs");
    test_cases.compile_fail("tests/ui/fail/*.rs");
}
