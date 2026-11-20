//! Fresh-process regression for HTTP transport feedback.
//!
//! Each ignored test owns the global Rust logger and the manager registry for
//! its process. A real HTTP handler sends to a local collector while `ureq`'s
//! DEBUG diagnostics are enabled through the same `log` bridge. The
//! application record and an independently emitted `ureq.run` record must
//! reach the collector exactly once each; diagnostics produced by the worker
//! must reach the non-HTTP sink but never the collector endpoint.

use std::num::NonZeroU64;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use _femtologging_rs::{
    ConfigBuilder, FemtoLevel, FemtoLogger, FileHandlerBuilder, HTTPHandlerBuilder, HTTPMethod,
    LoggerConfigBuilder, get_logger_for_log_compat, install_global_logger, manager,
};
use camino::Utf8Path;
use cap_std::ambient_authority;
use cap_std::fs_utf8::Dir;
use pyo3::Python;

use super::support::{RequestObservation, TestCollector};

/// Message identifying the ordinary application record.
const APPLICATION_MARKER: &str = "feedback-application-569";
/// Message identifying the bridge record emitted outside transport work.
const INDEPENDENT_MARKER: &str = "feedback-independent-569";
/// Request count above which the collector declares runaway feedback.
const HARD_REQUEST_CAP: usize = 32;
/// Time allowed for both expected requests to arrive.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);
/// Time to watch for requests that should never follow the expected two.
const QUIET_WINDOW: Duration = Duration::from_millis(500);
/// Per-logger flush budget in the child.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// Run the POST plus JSON variant of the regression.
#[test]
#[ignore = "run in a fresh process because the global logger and manager are process-global"]
fn post_json_transport_feedback_child() {
    run_feedback_child(HTTPMethod::POST, true);
}

/// Run the GET plus URL-encoded variant of the regression.
#[test]
#[ignore = "run in a fresh process because the global logger and manager are process-global"]
fn get_url_encoded_transport_feedback_child() {
    run_feedback_child(HTTPMethod::GET, false);
}

/// Configure the transport, emit both records, and verify network quiescence.
fn run_feedback_child(method: HTTPMethod, use_json: bool) {
    println!("child-start");
    let collector = TestCollector::spawn(HARD_REQUEST_CAP).expect("local collector should bind");
    let temp_dir = tempfile::tempdir().expect("collector directory should be created");
    let collector_path = temp_dir.path().join("non-http-records.log");
    let expected_method = method.as_str();

    install_child_configuration(&collector, &collector_path, method, use_json);
    println!("config-ready");

    log::info!(target: "test.application", "{APPLICATION_MARKER} application record");
    log::debug!(
        target: "ureq::run",
        "{INDEPENDENT_MARKER} independent bridge record"
    );

    let observed = wait_for_expected_records(&collector, Instant::now() + DELIVERY_TIMEOUT);
    assert_eq!(
        observed.len(),
        2,
        "the expected records should be the only deliveries: {observed:#?}"
    );
    assert_payloads_are_application_records(&observed, expected_method);

    println!("flush-begin");
    let (root_flushed, transport_flushed) = flush_loggers();
    assert!(
        root_flushed,
        "the root HTTP handler should acknowledge flush"
    );
    assert!(
        transport_flushed,
        "the non-HTTP collector should acknowledge flush"
    );
    println!("flush-end");

    assert_collected_non_http_records(&collector_path);
    assert_quiet_after_expected_deliveries(&collector);

    Python::attach(|_| manager::reset_manager());
    println!("close-end");
    collector
        .stop_and_join(Duration::from_secs(2))
        .expect("collector thread should stop");
    println!("server-stopped");
    println!("request-count=2");
    println!("child-complete");
}

/// Install the global bridge and configure the four loggers under test.
fn install_child_configuration(
    collector: &TestCollector,
    collector_path: &std::path::Path,
    method: HTTPMethod,
    use_json: bool,
) {
    Python::attach(|_| manager::reset_manager());
    assert!(
        install_global_logger(),
        "the child process owns the global Rust logger"
    );

    let url = format!("http://{}/collect", collector.addr());
    let mut http = HTTPHandlerBuilder::new()
        .with_url(url)
        .with_method(method)
        .with_connect_timeout_ms(FLUSH_TIMEOUT.as_millis() as u64)
        .with_write_timeout_ms(FLUSH_TIMEOUT.as_millis() as u64);
    if use_json {
        http = http.with_json_format();
    }

    let file = FileHandlerBuilder::new(collector_path)
        .with_flush_after_records(NonZeroU64::new(1).expect("1 is non-zero"));

    ConfigBuilder::new()
        .with_handler("collector", file)
        .with_handler("http", http)
        .with_root_logger(
            LoggerConfigBuilder::new()
                .with_handlers(["http"])
                .with_level(FemtoLevel::Info),
        )
        // ureq 3 emits the per-attempt diagnostics from `ureq::run` and the
        // connection-pool ones from `ureq::pool`; ureq 2's `ureq::unit` and
        // `ureq::stream` modules no longer exist.
        .with_logger(
            "ureq.run",
            LoggerConfigBuilder::new()
                .with_handlers(["collector"])
                .with_level(FemtoLevel::Debug)
                .with_propagate(true),
        )
        .with_logger(
            "ureq.pool",
            LoggerConfigBuilder::new()
                .with_level(FemtoLevel::Debug)
                .with_propagate(true),
        )
        .with_logger(
            "test.application",
            LoggerConfigBuilder::new()
                .with_level(FemtoLevel::Info)
                .with_propagate(true),
        )
        .build_and_init()
        .expect("child configuration should commit");
}

/// Wait until both expected records have reached the collector endpoint.
fn wait_for_expected_records(
    collector: &TestCollector,
    deadline: Instant,
) -> Vec<RequestObservation> {
    let mut observed: Vec<RequestObservation> = Vec::new();
    loop {
        assert!(
            !collector.has_overflowed(),
            "collector exceeded its hard request cap; observed: {observed:#?}"
        );
        let saw_application = observed.iter().any(|r| r.contains(APPLICATION_MARKER));
        let saw_independent = observed.iter().any(|r| r.contains(INDEPENDENT_MARKER));
        if saw_application && saw_independent {
            return observed;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for both expected records; observed: {observed:#?}"
        );
        match collector.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(request) => observed.push(request),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("collector stopped before both records arrived")
            }
        }
    }
}

/// Assert the method and payloads match ordinary application records.
fn assert_payloads_are_application_records(observed: &[RequestObservation], expected_method: &str) {
    for request in observed {
        assert_eq!(
            request.method, expected_method,
            "unexpected request method: {request:#?}"
        );
        let normalized = request.payload().replace('+', " ").to_ascii_lowercase();
        for diagnostic in TRANSPORT_DIAGNOSTICS {
            // The phrase list keeps its original casing for the sink check, so
            // it is lowered here: a mixed-case needle would never match the
            // lowered payload and the assertion could not fail.
            let lowered = diagnostic.to_ascii_lowercase();
            assert!(
                !normalized.contains(lowered.as_str()),
                "transport diagnostic {diagnostic:?} was delivered over HTTP: {request:#?}"
            );
        }
    }
}

/// Phrases `ureq` 3 emits at DEBUG from `ureq::run` while the worker sends.
///
/// ureq 3 logs the request construction as `Request { method: ... }` and the
/// received head as `Response { status: ... }`. The ureq 2 phrases this test
/// started with (`sending request`, `created stream`, and so on) came from
/// modules that no longer exist.
const TRANSPORT_DIAGNOSTICS: [&str; 2] = ["Request { method:", "Response { status:"];

/// Flush the root and transport loggers, reporting both outcomes.
fn flush_loggers() -> (bool, bool) {
    Python::attach(|py| {
        let root = get_logger_for_log_compat(py, "root").expect("root logger exists");
        let transport = get_logger_for_log_compat(py, "ureq.run").expect("transport logger exists");
        let root = root.borrow(py);
        let transport = transport.borrow(py);
        let root_ref: &FemtoLogger = &root;
        let transport_ref: &FemtoLogger = &transport;
        // The bridge's own diagnostics need the GIL while the worker is
        // finishing a delivery; holding it here would make the flush marker
        // wait behind that delivery until the timeout.
        py.detach(|| (root_ref.flush_handlers(), transport_ref.flush_handlers()))
    })
}

/// Read the collector log through a capability opened at its own directory.
///
/// The repository's Dylint filesystem policy disallows ambient `std::fs` even
/// in tests, so the read goes through a `cap_std` handle confined to the
/// directory holding the log rather than to the whole filesystem.
fn read_collected_log(path: &std::path::Path) -> String {
    let directory_path =
        Utf8Path::from_path(path).expect("collector log path should be valid UTF-8");
    let directory_root = directory_path
        .parent()
        .expect("collector log should have a parent directory");
    let file_name = directory_path
        .file_name()
        .expect("collector log should have a file name");
    let directory = Dir::open_ambient_dir(directory_root, ambient_authority())
        .expect("collector log directory should open");
    directory
        .read_to_string(file_name)
        .expect("collector log should be readable")
}

/// Assert the non-HTTP sink retained the independent and real diagnostics.
fn assert_collected_non_http_records(path: &std::path::Path) {
    let collected = read_collected_log(path);
    assert!(
        collected.contains(INDEPENDENT_MARKER),
        "the independent bridge record must reach the non-HTTP sink:\n{collected}"
    );
    let saw_transport_debug = collected.lines().any(|line| {
        line.contains("ureq.run")
            && TRANSPORT_DIAGNOSTICS
                .iter()
                .any(|diagnostic| line.contains(diagnostic))
    });
    assert!(
        saw_transport_debug,
        "the bridge should have collected real ureq DEBUG diagnostics:\n{collected}"
    );
}

/// Assert the server stays quiet once the two expected records are delivered.
fn assert_quiet_after_expected_deliveries(collector: &TestCollector) {
    let deadline = Instant::now() + QUIET_WINDOW;
    let mut unexpected = Vec::new();
    loop {
        assert!(
            !collector.has_overflowed(),
            "collector exceeded its hard request cap after the expected records"
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match collector.recv_timeout(remaining.min(Duration::from_millis(50))) {
            Ok(request) => unexpected.push(request),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    assert!(
        unexpected.is_empty(),
        "transport work continued after delivery: {unexpected:#?}"
    );
    assert_eq!(
        collector.request_count(),
        2,
        "exactly the application and independent records should be delivered"
    );
}
