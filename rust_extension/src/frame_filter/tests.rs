//! Tests for frame filtering utilities.

use super::*;
use rstest::rstest;

fn make_frame(filename: &str, lineno: u32, function: &str) -> StackFrame {
    StackFrame::new(filename, lineno, function)
}

/// Generic helper to assert filtered frames have expected length and field values.
fn assert_filter_result_by_field<F>(
    filtered: &[StackFrame],
    expected_len: usize,
    expected_values: &[&str],
    field_extractor: F,
) where
    F: Fn(&StackFrame) -> &str,
{
    assert_eq!(
        expected_len,
        expected_values.len(),
        "expected_len ({}) must match expected_values.len() ({})",
        expected_len,
        expected_values.len()
    );
    assert_eq!(filtered.len(), expected_len);
    for (i, expected) in expected_values.iter().enumerate() {
        assert_eq!(
            field_extractor(&filtered[i]),
            *expected,
            "Mismatch at index {}",
            i
        );
    }
}

/// Assert filtered frames have expected length and filenames.
fn assert_filter_result(filtered: &[StackFrame], expected_len: usize, expected_filenames: &[&str]) {
    assert_filter_result_by_field(filtered, expected_len, expected_filenames, |f| &f.filename);
}

/// Assert filtered frames have expected length and function names.
fn assert_filter_result_by_function(
    filtered: &[StackFrame],
    expected_len: usize,
    expected_functions: &[&str],
) {
    assert_filter_result_by_field(filtered, expected_len, expected_functions, |f| &f.function);
}

#[rstest]
fn filter_frames_with_predicate() {
    let frames = vec![
        make_frame("a.py", 1, "func_a"),
        make_frame("b.py", 2, "func_b"),
        make_frame("c.py", 3, "func_c"),
    ];

    let filtered = filter_frames(&frames, |f| f.filename != "b.py");

    assert_filter_result(&filtered, 2, &["a.py", "c.py"]);
}

#[rstest]
fn filter_frames_empty_input() {
    let frames: Vec<StackFrame> = vec![];
    let filtered = filter_frames(&frames, |_| true);
    assert!(filtered.is_empty());
}

#[rstest]
fn filter_frames_all_excluded() {
    let frames = vec![make_frame("a.py", 1, "func")];
    let filtered = filter_frames(&frames, |_| false);
    assert!(filtered.is_empty());
}

#[rstest]
fn limit_frames_under_limit() {
    let frames = vec![make_frame("a.py", 1, "a"), make_frame("b.py", 2, "b")];

    let limited = limit_frames(&frames, 5);

    assert_eq!(limited.len(), 2);
}

#[rstest]
fn limit_frames_at_limit() {
    let frames = vec![make_frame("a.py", 1, "a"), make_frame("b.py", 2, "b")];

    let limited = limit_frames(&frames, 2);

    assert_eq!(limited.len(), 2);
}

#[rstest]
fn limit_frames_over_limit() {
    let frames = vec![
        make_frame("a.py", 1, "outer"),
        make_frame("b.py", 2, "middle"),
        make_frame("c.py", 3, "inner"),
    ];

    let limited = limit_frames(&frames, 2);

    assert_filter_result(&limited, 2, &["b.py", "c.py"]);
}

#[rstest]
fn limit_frames_zero() {
    let frames = vec![make_frame("a.py", 1, "a")];
    let limited = limit_frames(&frames, 0);
    assert!(limited.is_empty());
}

#[rstest]
fn exclude_by_filename_single_pattern() {
    let frames = vec![
        make_frame("app/main.py", 1, "main"),
        make_frame(".venv/lib/foo.py", 2, "foo"),
        make_frame("app/utils.py", 3, "utils"),
    ];

    let filtered = exclude_by_filename(&frames, &[".venv/"]);

    assert_filter_result(&filtered, 2, &["app/main.py", "app/utils.py"]);
}

#[rstest]
fn exclude_by_filename_multiple_patterns() {
    let frames = vec![
        make_frame("app/main.py", 1, "main"),
        make_frame(".venv/lib/foo.py", 2, "foo"),
        make_frame("site-packages/bar.py", 3, "bar"),
    ];

    let filtered = exclude_by_filename(&frames, &[".venv/", "site-packages/"]);

    assert_filter_result(&filtered, 1, &["app/main.py"]);
}

#[rstest]
fn exclude_by_filename_no_matches() {
    let frames = vec![
        make_frame("app/main.py", 1, "main"),
        make_frame("app/utils.py", 2, "utils"),
    ];

    let filtered = exclude_by_filename(&frames, &[".venv/"]);

    assert_eq!(filtered.len(), 2);
}

#[rstest]
fn exclude_by_function_single_pattern() {
    let frames = vec![
        make_frame("app.py", 1, "main"),
        make_frame("app.py", 2, "_private_helper"),
        make_frame("app.py", 3, "public_api"),
    ];

    let filtered = exclude_by_function(&frames, &["_private"]);

    assert_filter_result_by_function(&filtered, 2, &["main", "public_api"]);
}

#[rstest]
fn exclude_by_function_multiple_patterns() {
    let frames = vec![
        make_frame("app.py", 1, "main"),
        make_frame("app.py", 2, "_private_helper"),
        make_frame("app.py", 3, "__dunder_method"),
        make_frame("app.py", 4, "public_api"),
    ];

    let filtered = exclude_by_function(&frames, &["_private", "__dunder"]);

    assert_filter_result_by_function(&filtered, 2, &["main", "public_api"]);
}

#[rstest]
fn exclude_by_function_no_matches() {
    let frames = vec![
        make_frame("app.py", 1, "main"),
        make_frame("app.py", 2, "public_api"),
        make_frame("app.py", 3, "helper"),
    ];

    let filtered = exclude_by_function(&frames, &["_private", "__internal"]);

    assert_filter_result_by_function(&filtered, 3, &["main", "public_api", "helper"]);
}

#[rstest]
fn exclude_logging_infrastructure_removes_femtologging() {
    let frames = vec![
        make_frame("myapp/main.py", 10, "run"),
        make_frame("femtologging/__init__.py", 50, "info"),
    ];

    let filtered = exclude_logging_infrastructure(&frames);

    assert_filter_result(&filtered, 1, &["myapp/main.py"]);
}

#[rstest]
fn exclude_logging_infrastructure_removes_standard_logging() {
    let frames = vec![
        make_frame("myapp/main.py", 10, "run"),
        make_frame("/usr/lib/python3.11/logging/__init__.py", 100, "_log"),
    ];

    let filtered = exclude_logging_infrastructure(&frames);

    assert_eq!(filtered.len(), 1);
}

#[rstest]
fn exclude_logging_infrastructure_removes_rust_extension() {
    let frames = vec![
        make_frame("myapp/main.py", 10, "run"),
        make_frame("_femtologging_rs.cpython-311-x86_64-linux-gnu.so", 0, "log"),
    ];

    let filtered = exclude_logging_infrastructure(&frames);

    assert_eq!(filtered.len(), 1);
}

#[rstest]
fn exclude_logging_infrastructure_removes_import_machinery() {
    let frames = vec![
        make_frame("myapp/main.py", 10, "run"),
        make_frame(
            "<frozen importlib._bootstrap>",
            0,
            "_call_with_frames_removed",
        ),
    ];

    let filtered = exclude_logging_infrastructure(&frames);

    assert_eq!(filtered.len(), 1);
}

#[rstest]
#[case("femtologging/__init__.py", true)]
#[case("logging/__init__.py", true)]
#[case("_femtologging_rs.so", true)]
#[case("myapp/main.py", false)]
#[case("/usr/lib/python3.11/logging/handlers.py", true)]
#[case("<frozen importlib._bootstrap>", true)]
fn is_logging_infrastructure_detects_patterns(#[case] filename: &str, #[case] expected: bool) {
    let frame = make_frame(filename, 1, "func");
    assert_eq!(
        is_logging_infrastructure(&frame),
        expected,
        "Expected is_logging_infrastructure('{}') to be {}",
        filename,
        expected
    );
}

#[rstest]
fn combined_filtering() {
    let frames = vec![
        make_frame("outer.py", 1, "start"),
        make_frame(".venv/lib/requests.py", 2, "get"),
        make_frame("myapp/api.py", 3, "fetch"),
        make_frame("femtologging/__init__.py", 4, "error"),
        make_frame("myapp/handler.py", 5, "handle"),
    ];

    // First exclude logging infrastructure
    let step1 = exclude_logging_infrastructure(&frames);
    assert_eq!(step1.len(), 4);

    // Then exclude virtualenv
    let step2 = exclude_by_filename(&step1, &[".venv/"]);
    assert_eq!(step2.len(), 3);

    // Finally limit depth
    let step3 = limit_frames(&step2, 2);
    assert_filter_result(&step3, 2, &["myapp/api.py", "myapp/handler.py"]);
}
