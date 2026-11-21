//! Unit tests for the message shapes built without running user code.

use pyo3::types::PyDict;
use rstest::rstest;

use super::*;

/// Build a `RenderedArgs` from `texts` and a larger total.
fn rendered(texts: &[&str], total: usize) -> RenderedArgs {
    RenderedArgs {
        texts: texts.iter().map(|text| (*text).to_string()).collect(),
        total,
        ..RenderedArgs::none()
    }
}

#[rstest]
#[case(&["'a'", "'b'"], 2, "('a', 'b')")]
#[case(&["'a'"], 5, "('a', ...)")]
#[case(&[], 5, "(...)")]
fn argument_tuples_render_as_a_prefix(
    #[case] texts: &[&str],
    #[case] total: usize,
    #[case] expected: &str,
) {
    assert_eq!(joined_args(&rendered(texts, total)), expected.to_string());
}

#[test]
fn a_failed_representation_in_the_middle_leaves_a_gap() {
    let args = RenderedArgs {
        texts: vec!["'a'".to_string(), "'c'".to_string()],
        total: 3,
        failed: 1,
        first_failed: Some(1),
        truncated: false,
        first_arg: None,
    };

    // Later texts stay in `args_repr`, but the message must not move `'c'`
    // into the failed argument's position.
    assert_eq!(joined_args(&args), "('a', ...)".to_string());
}

#[test]
fn failed_renders_do_not_mark_unretained_args_as_truncated() {
    let args = RenderedArgs {
        texts: Vec::new(),
        total: 0,
        failed: 3,
        truncated: true,
        ..RenderedArgs::none()
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

#[test]
fn a_keyerror_with_a_failing_key_repr_does_not_call_it_again() {
    Python::attach(|py| {
        let globals = PyDict::new(py);
        let error = py
            .run(
                c"
calls = 0

class Hostile:
    def __repr__(self):
        global calls
        calls += 1
        raise RuntimeError('no repr')

raise KeyError(Hostile())
",
                Some(&globals),
                None,
            )
            .expect_err("source should raise");
        let calls = |globals: &Bound<'_, PyDict>| -> usize {
            globals
                .get_item("calls")
                .expect("counter lookup should succeed")
                .expect("counter should be defined")
                .extract()
                .expect("counter should be an integer")
        };
        let exception = error.value(py);
        let limits = CaptureLimits::default();
        let rendered = render_args(exception.as_any(), &limits, true);
        let calls_after_render = calls(&globals);
        let message = build_message(exception.as_any(), &rendered, limits.max_message_bytes);

        assert_eq!(
            calls_after_render, 1,
            "the key's representation must run once"
        );
        assert_eq!(
            message.text, "<repr conversion failed>",
            "the message must report the contained failure"
        );
        assert_eq!(
            calls(&globals),
            1,
            "building the message must not retry the failing repr"
        );
    });
}

#[test]
fn a_shortened_keyerror_representation_marks_the_message_as_truncated() {
    Python::attach(|py| {
        let error = py
            .run(c"raise KeyError('k' * 600)", None, None)
            .expect_err("source should raise");
        let exception = error.value(py);
        let limits = CaptureLimits {
            max_arg_bytes: 16,
            max_message_bytes: 32,
            ..CaptureLimits::default()
        };
        let rendered = render_args(exception.as_any(), &limits, true);
        let message = build_message(exception.as_any(), &rendered, limits.max_message_bytes);

        assert!(
            message.text.len() <= 32,
            "the message must stay inside its ceiling: {:?}",
            message.text
        );
        assert!(
            message.truncated,
            "a shortened key representation shortens the message too"
        );
    });
}
