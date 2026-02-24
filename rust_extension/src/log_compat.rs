//! Compatibility bridge for the Rust `log` crate.
//!
//! This module provides `FemtoLogAdapter`, an implementation of
//! `log::Log` that forwards Rust-side log records into femtologging's
//! asynchronous handler pipeline. The bridge is enabled explicitly from
//! Python via `setup_rust_logging()`, which installs the adapter as the
//! global Rust logger.

use std::borrow::Cow;
use std::sync::OnceLock;

use log::{Metadata, Record};
use pyo3::prelude::*;

use crate::level::FemtoLevel;
use crate::log_record::{FemtoLogRecord, RecordMetadata};
use crate::manager;

/// Adapter implementing the Rust `log::Log` trait.
///
/// The adapter resolves a femtologging logger based on each record's target,
/// converts the record to a [`FemtoLogRecord`], and dispatches it through the
/// logger's handler queue.
pub struct FemtoLogAdapter;

fn map_log_level(level: log::Level) -> FemtoLevel {
    match level {
        log::Level::Trace => FemtoLevel::Trace,
        log::Level::Debug => FemtoLevel::Debug,
        log::Level::Info => FemtoLevel::Info,
        log::Level::Warn => FemtoLevel::Warn,
        log::Level::Error => FemtoLevel::Error,
    }
}

fn map_femto_to_log_level(level: FemtoLevel) -> log::Level {
    match level {
        FemtoLevel::Trace => log::Level::Trace,
        FemtoLevel::Debug => log::Level::Debug,
        FemtoLevel::Info => log::Level::Info,
        FemtoLevel::Warn => log::Level::Warn,
        FemtoLevel::Error | FemtoLevel::Critical => log::Level::Error,
    }
}

impl From<log::Level> for FemtoLevel {
    fn from(level: log::Level) -> Self {
        map_log_level(level)
    }
}

fn normalize_target(target: &str) -> Cow<'_, str> {
    if target.contains("::") {
        Cow::Owned(target.replace("::", "."))
    } else {
        Cow::Borrowed(target)
    }
}

fn is_unknown_logger_error(py: Python<'_>, err: &PyErr) -> bool {
    err.is_instance_of::<pyo3::exceptions::PyKeyError>(py)
}

fn is_invalid_logger_error(py: Python<'_>, err: &PyErr) -> bool {
    err.is_instance_of::<pyo3::exceptions::PyValueError>(py)
}

/// Classify a logger resolution error for diagnostic logging.
fn classify_logger_error(py: Python<'_>, err: &PyErr) -> &'static str {
    if is_unknown_logger_error(py, err) {
        "unknown logger target"
    } else if is_invalid_logger_error(py, err) {
        "invalid logger target"
    } else {
        "unexpected error resolving logger"
    }
}

fn resolve_logger<'py>(py: Python<'py>, target: &str) -> Option<(String, Py<crate::FemtoLogger>)> {
    let normalized = normalize_target(target);
    match manager::get_logger(py, normalized.as_ref()) {
        Ok(logger) => Some((normalized.into_owned(), logger)),
        Err(err) => {
            let reason = classify_logger_error(py, &err);
            log::warn!(
                target: "femtologging.log_compat",
                "femtologging: {reason} {:?} (normalized {:?}); falling back to root: {}",
                target,
                normalized.as_ref(),
                err
            );
            let logger = manager::get_logger(py, "root").ok()?;
            Some(("root".to_string(), logger))
        }
    }
}

fn is_enabled_by_global_max(level: log::Level) -> bool {
    log::max_level() >= level.to_level_filter()
}

impl log::Log for FemtoLogAdapter {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        is_enabled_by_global_max(metadata.level())
    }

    fn log(&self, record: &Record<'_>) {
        if !is_enabled_by_global_max(record.level()) {
            return;
        }

        Python::attach(|py| {
            let Some((logger_name, logger)) = resolve_logger(py, record.target()) else {
                return;
            };

            let level = FemtoLevel::from(record.level());
            if !logger.borrow(py).is_enabled_for(level) {
                return;
            }

            let metadata = RecordMetadata {
                module_path: record.module_path().unwrap_or_default().to_string(),
                filename: record.file().unwrap_or_default().to_string(),
                line_number: record.line().unwrap_or(0),
                ..Default::default()
            };

            let femto_record = FemtoLogRecord::with_metadata(
                logger_name.as_str(),
                level,
                &record.args().to_string(),
                metadata,
            );

            logger.borrow(py).dispatch_record(femto_record);
        });
    }

    fn flush(&self) {
        Python::attach(|py| {
            manager::flush_all_handlers(py);
        });
    }
}

static FEMTO_LOG_ADAPTER: FemtoLogAdapter = FemtoLogAdapter;
static INSTALL_RESULT: OnceLock<bool> = OnceLock::new();

/// Install femtologging as the global Rust logger.
///
/// Returns `true` on success. When a different global logger is already set,
/// installation fails and `false` is returned. Subsequent calls return the
/// cached outcome.
pub(crate) fn install_global_logger() -> bool {
    *INSTALL_RESULT.get_or_init(|| {
        if log::set_logger(&FEMTO_LOG_ADAPTER).is_err() {
            return false;
        }
        log::set_max_level(log::LevelFilter::Trace);
        true
    })
}

/// Python-facing entrypoint for enabling the `log` crate bridge.
///
/// Installs the adapter as the global Rust logger. The operation is
/// idempotent: repeated calls after a successful install are no-ops.
#[cfg(feature = "python")]
#[pyfunction]
pub(crate) fn setup_rust_logging() -> PyResult<()> {
    if install_global_logger() {
        Ok(())
    } else {
        Err(pyo3::exceptions::PyRuntimeError::new_err(
            "global Rust logger is already set; femtologging cannot install the log bridge",
        ))
    }
}

/// Emit a Rust log record via the `log` crate.
///
/// This is an internal helper used by the Python behavioural tests to validate
/// the bridge. `CRITICAL` maps to `ERROR` because the `log` crate has no
/// critical level.
#[cfg(feature = "python")]
#[pyfunction]
#[pyo3(name = "_emit_rust_log")]
pub(crate) fn emit_rust_log(level: FemtoLevel, message: &str, target: Option<&str>) {
    let mapped = map_femto_to_log_level(level);

    if let Some(target) = target {
        log::log!(target: target, mapped, "{}", message);
    } else {
        log::log!(mapped, "{}", message);
    }
}

/// Install a dummy global Rust logger for behavioural tests.
///
/// This helper is intended for subprocess-based test scenarios that need to
/// verify `setup_rust_logging()` fails when a different global logger has
/// already been configured.
#[cfg(feature = "python")]
#[pyfunction]
#[pyo3(name = "_install_test_global_rust_logger")]
pub(crate) fn install_test_global_rust_logger() -> PyResult<()> {
    struct TestLogger;

    impl log::Log for TestLogger {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn log(&self, _record: &Record<'_>) {}

        fn flush(&self) {}
    }

    static TEST_LOGGER: TestLogger = TestLogger;
    log::set_logger(&TEST_LOGGER).map_err(|_| {
        pyo3::exceptions::PyRuntimeError::new_err("global Rust logger is already set")
    })?;
    log::set_max_level(log::LevelFilter::Trace);
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Unit tests for the `log` crate bridge.

    use log::{LevelFilter, Log};

    use super::*;
    use crate::handler::{FemtoHandlerTrait, HandlerError};
    use parking_lot::Mutex;
    use rstest::{fixture, rstest};
    use std::any::Any;
    use std::sync::{
        Arc, Once,
        atomic::{AtomicUsize, Ordering},
    };

    static LOGGER_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[fixture]
    fn unique_logger_name() -> String {
        let base = "bridge.test";
        let suffix = LOGGER_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{base}.{suffix}")
    }

    #[rstest]
    #[case(log::Level::Trace, FemtoLevel::Trace)]
    #[case(log::Level::Debug, FemtoLevel::Debug)]
    #[case(log::Level::Info, FemtoLevel::Info)]
    #[case(log::Level::Warn, FemtoLevel::Warn)]
    #[case(log::Level::Error, FemtoLevel::Error)]
    fn level_mapping_is_direct(#[case] level: log::Level, #[case] expected: FemtoLevel) {
        assert_eq!(FemtoLevel::from(level), expected);
    }

    #[derive(Clone, Default)]
    struct CollectingHandler {
        records: Arc<Mutex<Vec<FemtoLogRecord>>>,
    }

    #[fixture]
    fn log_max_level() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            log::set_max_level(LevelFilter::Trace);
        });
    }

    impl CollectingHandler {
        fn collected(&self) -> Vec<FemtoLogRecord> {
            self.records.lock().clone()
        }
    }

    impl FemtoHandlerTrait for CollectingHandler {
        fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
            self.records.lock().push(record);
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[rstest]
    fn adapter_dispatches_records_to_target_logger(_log_max_level: (), unique_logger_name: String) {
        let adapter = FemtoLogAdapter;
        let logger_name = unique_logger_name;

        Python::attach(|py| {
            let logger = manager::get_logger(py, &logger_name).expect("logger created");
            let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
            logger.borrow(py).add_handler(handler.clone());

            let record = log::Record::builder()
                .args(format_args!("hello"))
                .level(log::Level::Info)
                .target(&logger_name)
                .module_path(Some("bridge::test"))
                .file(Some("lib.rs"))
                .line(Some(42))
                .build();

            adapter.log(&record);

            assert!(
                logger.borrow(py).flush_handlers(),
                "flush should drain the queue"
            );

            let records = handler
                .as_any()
                .downcast_ref::<CollectingHandler>()
                .expect("handler downcast")
                .collected();
            assert_eq!(records.len(), 1);
            let rec = &records[0];
            assert_eq!(rec.logger(), logger_name.as_str());
            assert_eq!(rec.level_str(), "INFO");
            assert_eq!(rec.message(), "hello");
            assert_eq!(rec.metadata().module_path, "bridge::test");
            assert_eq!(rec.metadata().filename, "lib.rs");
            assert_eq!(rec.metadata().line_number, 42);
        });
    }

    #[rstest]
    fn adapter_normalizes_rust_module_targets(_log_max_level: (), unique_logger_name: String) {
        let adapter = FemtoLogAdapter;
        let logger_name = unique_logger_name;
        let target = logger_name.replace('.', "::");

        Python::attach(|py| {
            let logger = manager::get_logger(py, &logger_name).expect("logger created");
            let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
            logger.borrow(py).add_handler(handler.clone());

            let record = log::Record::builder()
                .args(format_args!("normalized"))
                .level(log::Level::Info)
                .target(&target)
                .build();

            adapter.log(&record);
            assert!(logger.borrow(py).flush_handlers());

            let records = handler
                .as_any()
                .downcast_ref::<CollectingHandler>()
                .expect("handler downcast")
                .collected();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].logger(), logger_name.as_str());
        });
    }

    #[rstest]
    fn log_respects_logger_threshold(_log_max_level: (), unique_logger_name: String) {
        let adapter = FemtoLogAdapter;
        let logger_name = unique_logger_name;

        Python::attach(|py| {
            let logger = manager::get_logger(py, &logger_name).expect("logger created");
            let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
            logger.borrow(py).add_handler(handler.clone());
            logger.borrow(py).set_level(FemtoLevel::Warn);

            let info_record = log::Record::builder()
                .args(format_args!("info"))
                .level(log::Level::Info)
                .target(&logger_name)
                .build();
            adapter.log(&info_record);

            let warn_record = log::Record::builder()
                .args(format_args!("warn"))
                .level(log::Level::Warn)
                .target(&logger_name)
                .build();
            adapter.log(&warn_record);

            assert!(
                logger.borrow(py).flush_handlers(),
                "flush should drain the queue"
            );

            let records = handler
                .as_any()
                .downcast_ref::<CollectingHandler>()
                .expect("handler downcast")
                .collected();
            assert_eq!(records.len(), 1, "only WARN should pass threshold");
            assert_eq!(records[0].level_str(), "WARN");
        });
    }
}
