//! Tests for the diagnostics a Python handler's flush reports.
//!
//! A failed flush is reported by a `bool`, a bounded warning and, with the
//! `tracing-compat` feature, a span and a trace event. The boolean alone tells
//! an operator nothing about which handler failed, so these tests pin the
//! fields the warning, the span, and the event carry. Each assertion names the
//! field it depends on, so removing, renaming, or constant-folding one fails
//! the test rather than passing silently.

use log::Level;
use pyo3::prelude::*;
use rstest::rstest;
use serial_test::serial;

use super::{NativeHandlerKind, PyHandler};
use crate::handler::{FemtoHandlerTrait, FlushContext};
use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};
use crate::{FemtoFileHandler, FemtoSocketHandler, SocketHandlerConfig};
use std::time::{Duration, Instant};

#[cfg(feature = "tracing-compat")]
use std::sync::Arc;
#[cfg(feature = "tracing-compat")]
use tracing_subscriber::prelude::*;

/// A built-in handler whose worker has already stopped, so flushing it fails.
#[derive(Clone, Copy)]
enum ClosedKind {
    /// A closed file handler.
    File,
    /// A closed socket handler.
    Socket,
}

impl ClosedKind {
    /// The label the flush diagnostics must use for this handler kind.
    const fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Socket => "socket",
        }
    }
}

/// Wrap a built-in handler whose worker has already stopped.
///
/// Both handlers report a failed flush once closed, which is what makes them
/// reach the failure diagnostics without needing a stalled peer.
fn closed_native_handler(py: Python<'_>, kind: ClosedKind) -> Py<PyAny> {
    match kind {
        ClosedKind::File => {
            let dir = tempfile::tempdir().expect("temporary directory");
            let handler = FemtoFileHandler::new(dir.path().join("app.log"))
                .expect("file handler should open");
            handler.close();
            Py::new(py, handler)
                .expect("closed file handler should be constructible as a Python object")
                .into_any()
        }
        ClosedKind::Socket => {
            let mut handler = FemtoSocketHandler::with_config(SocketHandlerConfig::default());
            handler.close();
            Py::new(py, handler)
                .expect("closed socket handler should be constructible as a Python object")
                .into_any()
        }
    }
}

/// Assert that the flush warning for `handler_kind` and `category` was captured.
///
/// The whole message is compared, so a changed field name, a reordered pair, a
/// different level, or a label that stopped describing the handler fails here.
fn assert_flush_warning(handler_kind: &str, category: &str, scenario: &str) {
    let expected =
        format!("PyHandler: flush failed: handler_kind={handler_kind}, category={category}");
    let captured: Vec<_> = take_logged_messages()
        .into_iter()
        .filter(|entry| entry.message.starts_with("PyHandler: flush failed"))
        .collect();
    assert!(
        captured
            .iter()
            .any(|entry| entry.level == Level::Warn && entry.message == expected),
        "{scenario} should warn {expected:?}, but the captured flush warnings were {:?}",
        captured
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>()
    );
}

/// A failed native flush names the handler kind and the failure category.
///
/// The cases use different built-in handlers, so a label that is a constant
/// rather than derived from the handler fails at least one of them.
#[rstest]
#[case::file(ClosedKind::File)]
#[case::socket(ClosedKind::Socket)]
#[serial]
fn native_flush_failure_warns_with_the_handler_kind(#[case] kind: ClosedKind) {
    Python::attach(|py| {
        install_test_logger();
        let wrapper = PyHandler::new(py, closed_native_handler(py, kind));
        assert!(
            !wrapper.flush(),
            "a closed {} handler must report a failed flush",
            kind.label()
        );
        assert_flush_warning(kind.label(), "flush_failed", "a closed native handler");
    });
}

/// A failed native flush under the barrier names the handler kind too.
///
/// `flush_until` reaches the warning through the same helper as the legacy
/// path, so a `False` returned by `flush_handlers()` still points at the
/// built-in handler that declined rather than only at the call.
#[rstest]
#[case::file(ClosedKind::File)]
#[case::socket(ClosedKind::Socket)]
#[serial]
fn barrier_flush_failure_warns_with_the_handler_kind(#[case] kind: ClosedKind) {
    Python::attach(|py| {
        install_test_logger();
        let wrapper = PyHandler::new(py, closed_native_handler(py, kind));
        let started_at = Instant::now();
        let context = FlushContext::new(started_at, started_at + Duration::from_secs(2));
        assert!(
            !wrapper.flush_until(&context),
            "a closed {} handler must report a failed barrier flush",
            kind.label()
        );
        assert_flush_warning(
            kind.label(),
            "flush_failed",
            "a closed native handler under the barrier",
        );
    });
}

/// Every native handler kind has its own bounded label.
///
/// The labels are what the warning and the tracing fields carry, so a variant
/// folded into a shared value would make two handlers indistinguishable in a
/// failure report.
#[rstest]
#[case::stream(NativeHandlerKind::Stream, "stream")]
#[case::file(NativeHandlerKind::File, "file")]
#[case::rotating(NativeHandlerKind::RotatingFile, "rotating-file")]
#[case::timed_rotating(NativeHandlerKind::TimedRotatingFile, "timed-rotating-file")]
#[case::socket(NativeHandlerKind::Socket, "socket")]
#[case::http(NativeHandlerKind::Http, "http")]
fn native_handler_kinds_have_distinct_labels(#[case] kind: NativeHandlerKind, #[case] label: &str) {
    assert_eq!(kind.label(), label);
}

/// An unattachable interpreter fails the registered flush and warns.
///
/// `flush_registered_handler` delegates to [`PyHandler::flush_with_interpreter`],
/// so driving that seam reports through the same path `flush_handlers` uses.
/// `Python::try_attach` returns `None` only while the interpreter is
/// finalizing, which no test can arrange, so the attempt is injected. Treating
/// that case as success would let `flush_handlers()` return `True` over records
/// nobody flushed.
#[test]
#[serial]
fn unavailable_interpreter_fails_the_registered_flush_with_a_warning() {
    Python::attach(|py| {
        install_test_logger();
        let wrapper = PyHandler::new(py, closed_native_handler(py, ClosedKind::File));
        assert!(
            !wrapper.flush_with_interpreter(|| None),
            "an unattachable interpreter must report a failed flush"
        );
        assert_flush_warning(
            "file",
            "interpreter_unavailable",
            "an unattachable interpreter",
        );
    });
}

#[cfg(feature = "tracing-compat")]
mod tracing_diagnostics {
    //! Capture of the flush span and the flush trace event.

    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, MutexGuard};

    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::layer::{Context, Layer};

    /// Rendered field values of one span or event.
    type Fields = BTreeMap<String, String>;

    /// Records fields into a map, keeping values rendered rather than typed.
    struct FieldVisitor<'a>(&'a mut Fields);

    impl Visit for FieldVisitor<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
    }

    /// What the test subscriber captured, in the order it was captured.
    #[derive(Default)]
    struct Captured {
        /// One entry per flush span, in creation order.
        spans: Vec<Fields>,
        /// Span identity mapped to its entry, so later records can find it.
        span_index: BTreeMap<u64, usize>,
        /// Every internal event, in emission order.
        events: Vec<Fields>,
    }

    /// Diagnostics captured by the test subscriber.
    #[derive(Default)]
    pub(super) struct FlushDiagnostics(Mutex<Captured>);

    impl FlushDiagnostics {
        /// Lock the captured state, recovering from poisoning.
        fn lock(&self) -> MutexGuard<'_, Captured> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }

        /// The captured flush spans, in creation order.
        pub(super) fn spans(&self) -> Vec<Fields> {
            self.lock().spans.clone()
        }

        /// The captured flush trace events, in emission order.
        pub(super) fn flush_events(&self) -> Vec<Fields> {
            self.lock()
                .events
                .iter()
                .filter(|fields| {
                    fields.get("operation").map(String::as_str) == Some("flush_registered_handler")
                })
                .cloned()
                .collect()
        }
    }

    /// Subscriber layer that records the flush span and internal events.
    pub(super) struct DiagnosticsLayer(Arc<FlushDiagnostics>);

    impl DiagnosticsLayer {
        /// Build a layer that records into `diagnostics`.
        pub(super) fn new(diagnostics: Arc<FlushDiagnostics>) -> Self {
            Self(diagnostics)
        }
    }

    impl<S: Subscriber> Layer<S> for DiagnosticsLayer {
        fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _context: Context<'_, S>) {
            if attrs.metadata().name() != "flush_registered_handler" {
                return;
            }
            let mut fields = Fields::new();
            attrs.record(&mut FieldVisitor(&mut fields));
            let mut captured = self.0.lock();
            let index = captured.spans.len();
            captured.span_index.insert(id.into_u64(), index);
            captured.spans.push(fields);
        }

        fn on_record(&self, id: &Id, values: &Record<'_>, _context: Context<'_, S>) {
            let mut captured = self.0.lock();
            let Some(index) = captured.span_index.get(&id.into_u64()).copied() else {
                return;
            };
            values.record(&mut FieldVisitor(&mut captured.spans[index]));
        }

        fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
            if event.metadata().target() != "femtologging::internal" {
                return;
            }
            let mut fields = Fields::new();
            event.record(&mut FieldVisitor(&mut fields));
            self.0.lock().events.push(fields);
        }
    }
}

/// The flush span and trace event carry the handler kind and the outcome.
///
/// A span without `flush_outcome`, or an event without `handler_kind`, still
/// leaves the boolean correct, so only reading these surfaces shows the loss.
/// Both outcomes are exercised: an operator needs to tell a flush that
/// succeeded from one that did not.
#[cfg(feature = "tracing-compat")]
#[test]
#[serial]
fn flush_diagnostics_record_the_handler_kind_and_outcome() {
    let diagnostics = Arc::new(tracing_diagnostics::FlushDiagnostics::default());
    let subscriber = tracing_subscriber::registry::Registry::default().with(
        tracing_diagnostics::DiagnosticsLayer::new(Arc::clone(&diagnostics)),
    );
    Python::attach(|py| {
        let wrapper = PyHandler::new(py, closed_native_handler(py, ClosedKind::File));
        tracing::subscriber::with_default(subscriber, || {
            assert!(
                wrapper.flush_with_interpreter(|| Some(true)),
                "the injected success must be reported as success"
            );
            assert!(
                !wrapper.flush_with_interpreter(|| Some(false)),
                "the injected failure must be reported as failure"
            );
        });
    });

    let spans = diagnostics.spans();
    assert_eq!(spans.len(), 2, "each flush attempt opens one span");
    let span_outcomes: Vec<_> = spans
        .iter()
        .map(|fields| fields.get("handler_kind").map(String::as_str))
        .collect();
    assert_eq!(
        span_outcomes,
        [Some("file"), Some("file")],
        "every flush span must name the handler kind"
    );
    let recorded: Vec<_> = spans
        .iter()
        .map(|fields| fields.get("flush_outcome").map(String::as_str))
        .collect();
    assert_eq!(
        recorded,
        [Some("succeeded"), Some("failed")],
        "the span must record the outcome of its own flush"
    );

    let events = diagnostics.flush_events();
    assert_eq!(events.len(), 2, "each flush attempt emits one trace event");
    for (event, expected) in events.iter().zip(["succeeded", "failed"]) {
        assert_eq!(
            event.get("handler_kind").map(String::as_str),
            Some("file"),
            "the trace event must name the handler kind"
        );
        assert_eq!(
            event.get("flush_outcome").map(String::as_str),
            Some(expected),
            "the trace event must carry the flush outcome"
        );
    }
}
