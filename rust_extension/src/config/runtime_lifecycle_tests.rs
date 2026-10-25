//! Lifetime and churn tests for runtime registry pruning.
//!
//! Registry pruning is only half the fix for unreferenced definitions; the
//! other half is that a superseded object is actually *released*. These tests
//! observe release directly with `Arc::downgrade()` on the published registry
//! entry, so they cannot pass because a `Drop` happened somewhere else, and
//! they assert registry size separately so a shrinking registry cannot mask a
//! leaked object.
#![cfg(all(test, feature = "python"))]

use std::{
    path::Path,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use pyo3::Python;
use rstest::rstest;
use serial_test::serial;
use tempfile::TempDir;

use super::{
    ConfigBuilder, LoggerConfigBuilder, LoggerMutationBuilder, RuntimeConfigBuilder,
    test_utils::{
        gil_and_clean_manager,
        prune_test_support::{
            handler_registry_len, install_core_logger, published_handler, temp_dir,
        },
    },
    types::HandlerBuilder,
};
use crate::{
    FemtoLevel, FileHandlerBuilder, HTTPHandlerBuilder, SocketHandlerBuilder,
    handler::{FemtoHandlerTrait, HandlerError},
    log_record::FemtoLogRecord,
    manager,
    sync::{Receiver, Sender, bounded},
};

/// Rounds of rejected churn. Large enough to distinguish a constant registry
/// from one that grows linearly in the number of rejected mutations.
const CHURN_ROUNDS: usize = 32;

/// Upper bound for observing a deferred release. Matches the logger flush
/// timeout, which is the same bound the repository uses elsewhere for
/// "the worker has finished with this record".
const RELEASE_TIMEOUT: Duration = Duration::from_millis(2_000);

#[derive(Clone, Copy, Debug)]
/// Selects the resource-owning handler used by lifecycle churn tests.
enum HandlerKind {
    /// A file handler that would create a file during construction.
    File,
    /// A socket handler that owns a connection worker and queue.
    Socket,
    /// An HTTP handler that owns a request worker and queue.
    Http,
}

fn resource_handler(kind: HandlerKind, path: &Path) -> HandlerBuilder {
    match kind {
        HandlerKind::File => FileHandlerBuilder::new(path).into(),
        HandlerKind::Socket => SocketHandlerBuilder::new().with_tcp("127.0.0.1", 9).into(),
        HandlerKind::Http => HTTPHandlerBuilder::new()
            .with_url("http://127.0.0.1:9/log")
            .into(),
    }
}

/// Poll `gate` until it returns `true` or [`RELEASE_TIMEOUT`] elapses.
///
/// Returns the final value of `gate`. Polling rather than sleeping a fixed
/// interval keeps the happy path fast without risking a flake under load.
fn wait_until(mut gate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + RELEASE_TIMEOUT;
    loop {
        if gate() || Instant::now() >= deadline {
            return gate();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Assert that a weak reference is released, polling until the bound elapses.
fn assert_released(weak: &Weak<dyn FemtoHandlerTrait>, context: &str) {
    assert!(
        wait_until(|| weak.upgrade().is_none()),
        "{context}: the superseded handler should be released within {RELEASE_TIMEOUT:?}",
    );
}

#[rstest]
#[case::file(HandlerKind::File)]
#[case::socket(HandlerKind::Socket)]
#[case::http(HandlerKind::Http)]
#[serial]
fn rejected_resource_handler_churn_keeps_registry_constant(
    _gil_and_clean_manager: (),
    temp_dir: TempDir,
    #[case] kind: HandlerKind,
) {
    install_core_logger();
    let baseline = handler_registry_len();

    Python::attach(|_py| {
        for round in 0..CHURN_ROUNDS {
            let path = temp_dir.path().join(format!("orphan-{round}.log"));
            let err = RuntimeConfigBuilder::new()
                .with_handler("orphan", resource_handler(kind, &path))
                .apply()
                .expect_err("an unreferenced resource handler must be rejected");
            assert!(
                err.to_string().contains("orphan"),
                "round {round}: rejection must name the unreferenced handler, got {err}",
            );
        }

        assert_eq!(
            baseline,
            handler_registry_len(),
            "{CHURN_ROUNDS} rejected mutations must not grow the handler registry",
        );
        if matches!(kind, HandlerKind::File) {
            let leftover = std::fs::read_dir(temp_dir.path())
                .expect("the temporary directory must remain readable")
                .count();
            assert_eq!(
                0, leftover,
                "{CHURN_ROUNDS} rejected mutations must not create any file",
            );
        }
    });
}

#[rstest]
#[case::file(HandlerKind::File)]
#[case::socket(HandlerKind::Socket)]
#[case::http(HandlerKind::Http)]
#[serial]
fn replacing_resource_handler_id_releases_superseded_object(
    _gil_and_clean_manager: (),
    temp_dir: TempDir,
    #[case] kind: HandlerKind,
) {
    Python::attach(|_py| {
        ConfigBuilder::new()
            .with_handler(
                "log",
                resource_handler(kind, &temp_dir.path().join("first.log")),
            )
            .with_root_logger(LoggerConfigBuilder::new().with_level(FemtoLevel::Debug))
            .with_logger("core", LoggerConfigBuilder::new().with_handlers(["log"]))
            .build_and_init()
            .expect("initial build should succeed");

        let baseline = handler_registry_len();
        let superseded = published_handler("log").expect("the handler should be published");
        let weak = Arc::downgrade(&superseded);

        RuntimeConfigBuilder::new()
            .with_handler(
                "log",
                resource_handler(kind, &temp_dir.path().join("second.log")),
            )
            .apply()
            .expect("replacing a reachable resource handler id should succeed");

        let replacement = published_handler("log").expect("the replacement should be published");
        assert_eq!(
            baseline,
            handler_registry_len(),
            "replacing one resource handler must keep registry size constant",
        );
        assert!(
            !Arc::ptr_eq(&superseded, &replacement),
            "the registry must publish the replacement object under the replaced id",
        );
        drop(superseded);
        assert_released(&weak, "same-id resource handler replacement");
    });
}

#[rstest]
#[serial]
fn detaching_last_reference_releases_object(_gil_and_clean_manager: (), temp_dir: TempDir) {
    let path = temp_dir.path().join("detached.log");
    Python::attach(|py| {
        ConfigBuilder::new()
            .with_handler("log", FileHandlerBuilder::new(path))
            .with_root_logger(LoggerConfigBuilder::new().with_level(FemtoLevel::Debug))
            .with_logger("core", LoggerConfigBuilder::new().with_handlers(["log"]))
            .build_and_init()
            .expect("initial build should succeed");

        let attached = published_handler("log").expect("the handler should be published");
        let weak = Arc::downgrade(&attached);

        RuntimeConfigBuilder::new()
            .with_logger(
                "core",
                LoggerMutationBuilder::new().remove_handlers(["log"]),
            )
            .apply()
            .expect("removing the last reference should succeed");

        assert!(
            published_handler("log").is_none(),
            "a handler with no remaining reference must leave the registry",
        );

        // The logger and the published entry are the only other references.
        // Draining proves the worker no longer holds the record that captured
        // the handler.
        let logger = manager::get_logger(py, "core").expect("the core logger should exist");
        assert!(
            logger.borrow(py).flush_handlers(),
            "draining the logger queue should succeed before checking release",
        );
        drop(attached);
        assert_released(&weak, "detaching the last reference");
    });
}

/// A handler that parks its worker until the test opens the gate.
///
/// The runtime registry only holds `Arc<dyn FemtoHandlerTrait>`, so a test
/// cannot publish a bespoke type through configuration. Callers attach it
/// imperatively instead, which is the supported way to hold a caller-owned
/// handler object.
struct GatedHandler {
    /// Signals that the worker has entered this handler.
    entered: Sender<()>,
    /// Keeps the worker blocked until the test releases the record.
    release: Receiver<()>,
}

impl FemtoHandlerTrait for GatedHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        // A closed channel means the test has already finished with the gate;
        // proceeding is the right response.
        let _ = self.entered.send(());
        let _ = self.release.recv();
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[rstest]
#[serial]
fn in_flight_record_defers_release_until_worker_drains(
    _gil_and_clean_manager: (),
    temp_dir: TempDir,
) {
    let path = temp_dir.path().join("in-flight.log");
    Python::attach(|py| {
        ConfigBuilder::new()
            .with_handler("log", FileHandlerBuilder::new(path))
            .with_root_logger(LoggerConfigBuilder::new().with_level(FemtoLevel::Debug))
            .with_logger("core", LoggerConfigBuilder::new().with_handlers(["log"]))
            .build_and_init()
            .expect("initial build should succeed");

        let superseded = published_handler("log").expect("the handler should be published");
        let weak = Arc::downgrade(&superseded);

        // Park the worker mid-record. The queued record captured the logger's
        // handler list, so it now holds a strong reference to `superseded`.
        let (entered_tx, entered_rx) = bounded::<()>(1);
        let (release_tx, release_rx) = bounded::<()>(1);
        let logger = manager::get_logger(py, "core").expect("the core logger should exist");
        logger
            .borrow(py)
            .add_handler(Arc::new(GatedHandler {
                entered: entered_tx,
                release: release_rx,
            }))
            .expect("the gated handler should attach to the core logger");
        assert!(
            logger.borrow(py).log(FemtoLevel::Error, "parked").is_some(),
            "the record should be accepted before the worker parks",
        );
        assert!(
            entered_rx.recv_timeout(RELEASE_TIMEOUT).is_ok(),
            "the worker must reach the gated handler before the mutation is attempted",
        );

        // Replace the handler while the record is still in flight. The
        // registry and the live logger both stop referencing the old object.
        let replacement_path = temp_dir.path().join("replacement.log");
        RuntimeConfigBuilder::new()
            .with_handler("log", FileHandlerBuilder::new(replacement_path))
            .apply()
            .expect("replacing a reachable handler id should succeed");
        drop(superseded);

        assert!(
            weak.upgrade().is_some(),
            "an in-flight record must keep the superseded handler alive",
        );

        // Opening the gate lets the worker finish the record, after which the
        // captured handler list is released.
        drop(release_tx);
        assert!(
            logger.borrow(py).flush_handlers(),
            "draining the logger queue should succeed once the gate is open",
        );
        assert_released(&weak, "in-flight record drained");
    });
}
