//! Bounded regression coverage for HTTP transport feedback.
//!
//! The incident needs a real `ureq` DEBUG stream and a real worker thread, so
//! each case runs in a fresh process. The parent only owns the watchdog; the
//! child owns the global logger, the manager registry, the local collector and
//! every assertion about delivery.
//!
//! Regression: before transport-origin suppression, a diagnostic such as
//! `ureq::unit`'s "sending request" was routed back to the same HTTP handler.
//! Every delivery then manufactured more diagnostics, so the worker kept
//! sending until the collector was overwhelmed.

#![cfg(feature = "log-compat")]

use std::time::Duration;

#[path = "http_transport_feedback/child.rs"]
mod child;

#[path = "http_transport_feedback/support.rs"]
mod support;

use support::run_ignored_test;

/// Time allowed for a child to finish before the watchdog kills it.
const CHILD_TIMEOUT: Duration = Duration::from_secs(30);

/// The POST plus JSON child sends exactly its two application records.
#[test]
fn post_json_diagnostics_do_not_feed_their_own_handler() {
    assert_child_completes("child::post_json_transport_feedback_child");
}

/// The GET plus URL-encoded child observes the same quiescent delivery.
#[test]
fn get_url_encoded_diagnostics_do_not_feed_their_own_handler() {
    assert_child_completes("child::get_url_encoded_transport_feedback_child");
}

/// Run one child and require a clean exit with its completion marker.
fn assert_child_completes(test_name: &str) {
    let run = run_ignored_test(test_name, CHILD_TIMEOUT).expect("the child should start");
    assert!(run.succeeded(), "{}", run.failure_report());
    assert!(
        run.stdout.contains("child-complete"),
        "the child never reached its completion marker\n{}",
        run.failure_report()
    );
    assert!(
        run.stdout.contains("request-count=2"),
        "the child did not report exactly two delivered requests\n{}",
        run.failure_report()
    );
}
