//! Shared helpers for file handler tests.
//!
//! Provides a simple capturing logger so multiple tests can assert emitted log
//! messages without conflicting with the global logger state.

use std::sync::{Mutex, Once, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};

#[derive(Clone, Debug)]
pub(crate) struct CapturedLog {
    pub level: Level,
    pub message: String,
}

struct TestLogger;

static LOGGER: TestLogger = TestLogger;
static INIT: Once = Once::new();
static LOGS: OnceLock<Mutex<Vec<CapturedLog>>> = OnceLock::new();

impl Log for TestLogger {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            let logs = LOGS.get_or_init(|| Mutex::new(Vec::new()));
            // Recover from poisoning: captured logs remain valid data.
            let mut guard = logs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.push(CapturedLog {
                level: record.level(),
                message: record.args().to_string(),
            });
        }
    }

    fn flush(&self) {}
}

pub(crate) fn install_test_logger() {
    INIT.call_once(|| {
        // `Once` guarantees a single installation, so a failure here means
        // another logger was installed outside this helper; that breaks the
        // capture contract and is ignored deliberately (assertions on
        // captured logs will then fail with context).
        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(LevelFilter::Trace);
        }
    });
    clear_logs();
}

pub(crate) fn take_logged_messages() -> Vec<CapturedLog> {
    let logs = LOGS.get_or_init(|| Mutex::new(Vec::new()));
    // Recover from poisoning: captured logs remain valid data.
    let mut guard = logs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.drain(..).collect()
}

pub(crate) fn clear_logs() {
    if let Some(logs) = LOGS.get() {
        logs.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

/// Implement [`std::io::Seek`] for a test double whose writer stands in for
/// something unseekable (e.g. an in-memory buffer or channel-backed stub).
///
/// Every rotation-capable writer must implement `Seek`, but most test
/// doubles never need real seek support; they exist only to observe writes.
/// This macro provides a single `seek` implementation that always reports
/// [`std::io::ErrorKind::Unsupported`], so each test double states its type
/// name once instead of hand-writing the same stub repeatedly.
macro_rules! impl_unsupported_seek {
    ($ty:ty) => {
        impl std::io::Seek for $ty {
            fn seek(&mut self, _pos: std::io::SeekFrom) -> std::io::Result<u64> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    concat!("seek unsupported for ", stringify!($ty)),
                ))
            }
        }
    };
}

pub(crate) use impl_unsupported_seek;
