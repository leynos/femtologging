//! Contracts for the drop report a normal HTTP flush asks its worker to make.
//!
//! The handler and its worker own separate warners: the handler counts what its
//! queue refused, and the worker counts what delivery lost. The flush marker
//! carries only the handler-side count, so a worker-side drop the warning
//! interval suppressed would survive a normal flush unreported unless the
//! worker reports its own warner when it handles the marker. The order in which
//! it does so, relative to the acknowledgement, is pinned separately by
//! `worker_tests.rs`; these tests drive a real worker through the handler.
//!
//! Included with `#[path]` from `tests.rs`, so the mock collector helpers there
//! stay in scope. Kept out of `drop_reporting_tests.rs`, which is at the
//! file-size limit.

use std::net::TcpListener;
use std::time::Duration;

use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};
use crate::http_handler::FemtoHTTPHandler;
use crate::http_handler::config::HTTPHandlerConfig;

/// The cause-neutral report a worker flush emits for `count` pending drops.
fn neutral_drop_report(count: u64) -> String {
    format!("FemtoHTTPHandler dropped {count} records since the last report")
}

/// Count the captured messages equal to `expected`.
fn occurrences(messages: &[String], expected: &str) -> usize {
    messages
        .iter()
        .filter(|message| *message == expected)
        .count()
}

/// A normal flush reports the worker's own suppressed count.
///
/// Scenario: two records reach a collector that answers with a permanent error,
/// inside one warning interval. The first worker-side drop warns and opens the
/// interval; the second is suppressed. A normal flush then queues a marker
/// carrying only the handler-side count. Invariant: the flush succeeds, the
/// worker's suppressed count is reported exactly once, and a second flush with
/// nothing pending emits no such report.
///
/// The interval is pinned far beyond the test's lifetime, because
/// `with_config` reads the system clock: a due moment between the two adjacent
/// refusals would make the second report itself and leave the flush with
/// nothing to say. Two loopback requests do not approach an hour.
#[test]
#[serial_test::serial]
fn a_flush_reports_the_workers_suppressed_drop() {
    install_test_logger();
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral listener");
    let (address, requests) = super::spawn_retry_server(listener, vec![400, 400])
        .expect("a collector answering two permanent errors should start");
    let config = HTTPHandlerConfig {
        url: format!("http://{address}/log"),
        warn_interval: Duration::from_secs(3600),
        write_timeout: Duration::from_secs(5),
        ..HTTPHandlerConfig::default()
    };
    let mut handler = FemtoHTTPHandler::with_config(config);
    for message in ["first refused record", "second refused record"] {
        super::send_info_record(&handler, message).expect("the queue should accept the record");
    }
    for _ in 0..2 {
        requests
            .recv_timeout(Duration::from_secs(5))
            .expect("the worker should send both records to the collector");
    }
    let _ = take_logged_messages();
    assert_eq!(
        handler.warner.pending_drops(),
        0,
        concat!(
            "the handler-side warner must be empty, so the report can only be ",
            "the worker's"
        )
    );

    assert!(
        handler.flush(),
        "an open handler's flush should be acknowledged"
    );
    let reported: Vec<_> = take_logged_messages()
        .into_iter()
        .map(|entry| entry.message)
        .collect();
    assert_eq!(
        occurrences(&reported, &neutral_drop_report(1)),
        1,
        "the flush must report the worker's suppressed drop: {reported:?}"
    );

    assert!(
        handler.flush(),
        "a flush with nothing pending should also be acknowledged"
    );
    let repeated: Vec<_> = take_logged_messages()
        .into_iter()
        .map(|entry| entry.message)
        .collect();
    assert_eq!(
        occurrences(&repeated, &neutral_drop_report(1)),
        0,
        "a flush with no worker-side drops must emit no drop report: {repeated:?}"
    );

    handler.close();
}
