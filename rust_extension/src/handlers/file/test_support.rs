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
            let mut guard = logs.lock().expect("logger mutex poisoned");
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
        log::set_logger(&LOGGER).expect("set test logger");
        log::set_max_level(LevelFilter::Trace);
    });
    clear_logs();
}

pub(crate) fn take_logged_messages() -> Vec<CapturedLog> {
    let logs = LOGS.get_or_init(|| Mutex::new(Vec::new()));
    let mut guard = logs.lock().expect("logger mutex poisoned");
    guard.drain(..).collect()
}

pub(crate) fn clear_logs() {
    if let Some(logs) = LOGS.get() {
        logs.lock().expect("logger mutex poisoned").clear();
    }
}
