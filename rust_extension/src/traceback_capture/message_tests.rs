//! Unit tests for the message shapes built without running user code.

use super::*;

/// Build a `RenderedArgs` from `texts` and a larger total.
fn rendered(texts: &[&str], total: usize) -> RenderedArgs {
    RenderedArgs {
        texts: texts.iter().map(|text| (*text).to_string()).collect(),
        total,
        failed: 0,
        truncated: false,
        first_arg: None,
    }
}

#[test]
fn complete_arguments_render_as_a_tuple() {
    assert_eq!(
        joined_args(&rendered(&["'a'", "'b'"], 2)),
        "('a', 'b')".to_string()
    );
}

#[test]
fn dropped_arguments_are_shown_as_a_prefix() {
    assert_eq!(
        joined_args(&rendered(&["'a'"], 5)),
        "('a', ...)".to_string()
    );
}

#[test]
fn a_wholly_dropped_argument_list_still_reads_as_a_tuple() {
    assert_eq!(joined_args(&rendered(&[], 5)), "(...)".to_string());
}

#[test]
fn failed_renders_do_not_mark_unretained_args_as_truncated() {
    let args = RenderedArgs {
        texts: Vec::new(),
        total: 0,
        failed: 3,
        truncated: true,
        first_arg: None,
    };
    let mut marker = PayloadTruncation::default();
    args.report_failures(&mut marker);

    assert_eq!(marker.failed_elements, 3);
    assert!(!marker.args_truncated);
}

#[test]
fn unrenderable_arguments_are_not_also_counted_as_omitted() {
    Python::attach(|py| {
        let error = py
            .run(
                c"
class Hostile:
    def __repr__(self):
        raise RuntimeError('no repr')

raise ValueError('a', Hostile(), 'c')
",
                None,
                None,
            )
            .expect_err("source should raise");
        let limits = CaptureLimits::default();
        let exception = error.value(py);
        let rendered = render_args(exception.as_any(), &limits, true);
        let mut budget = CaptureBudget::new(&limits);
        let mut marker = PayloadTruncation::default();
        rendered.report_failures(&mut marker);

        let retained = retain_args(rendered, &mut budget, &mut marker);

        assert_eq!(retained.len(), 2);
        assert_eq!(
            marker.failed_elements, 1,
            "the raising repr should be reported as a failed conversion"
        );
        assert_eq!(
            marker.args_omitted, 0,
            "a failed conversion is not also an argument dropped by the ceiling"
        );
    });
}
