//! Tests for frame filtering utilities.

use super::*;
use crate::test_utils::frame_test_helpers::{assert_frames, assert_frames_by_function, make_frame};
use rstest::rstest;

#[rstest]
fn filter_frames_with_predicate() {
    let frames = vec![
        make_frame("a.py", 1, "func_a"),
        make_frame("b.py", 2, "func_b"),
        make_frame("c.py", 3, "func_c"),
    ];

    let filtered = filter_frames(&frames, |f| f.filename != "b.py");

    assert_frames(&filtered, 2, &["a.py", "c.py"]);
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

    assert_frames(&limited, 2, &["b.py", "c.py"]);
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

    assert_frames(&filtered, 2, &["app/main.py", "app/utils.py"]);
}

#[rstest]
fn exclude_by_filename_multiple_patterns() {
    let frames = vec![
        make_frame("app/main.py", 1, "main"),
        make_frame(".venv/lib/foo.py", 2, "foo"),
        make_frame("site-packages/bar.py", 3, "bar"),
    ];

    let filtered = exclude_by_filename(&frames, &[".venv/", "site-packages/"]);

    assert_frames(&filtered, 1, &["app/main.py"]);
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
#[case(
    &["main", "_private_helper", "public_api"],
    &["_private"],
    &["main", "public_api"],
    "single pattern"
)]
#[case(
    &["main", "_private_helper", "__dunder_method", "public_api"],
    &["_private", "__dunder"],
    &["main", "public_api"],
    "multiple patterns"
)]
#[case(
    &["main", "public_api", "helper"],
    &["_private", "__internal"],
    &["main", "public_api", "helper"],
    "no matches"
)]
fn exclude_by_function_scenarios(
    #[case] function_names: &[&str],
    #[case] patterns: &[&str],
    #[case] expected_functions: &[&str],
    #[case] _scenario: &str,
) {
    let frames: Vec<StackFrame> = function_names
        .iter()
        .enumerate()
        .map(|(i, name)| make_frame("app.py", (i + 1) as u32, name))
        .collect();

    let filtered = exclude_by_function(&frames, patterns);

    assert_frames_by_function(&filtered, expected_functions.len(), expected_functions);
}

#[rstest]
fn exclude_logging_infrastructure_removes_femtologging() {
    let frames = vec![
        make_frame("myapp/main.py", 10, "run"),
        make_frame("femtologging/__init__.py", 50, "info"),
    ];

    let filtered = exclude_logging_infrastructure(&frames);

    assert_frames(&filtered, 1, &["myapp/main.py"]);
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
    assert_frames(&step3, 2, &["myapp/api.py", "myapp/handler.py"]);
}
