//! Fixtures shared by the `log` bridge's unit test modules.
//!
//! The bridge is covered by several sibling test modules, and each needs the
//! same two facilities: a logger name no other test in the binary already
//! holds, and a facade maximum permissive enough for a direct
//! [`log::Log::log`] call to reach the adapter.

use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};

use rstest::fixture;

static LOGGER_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Supply a logger name unique within this test binary.
///
/// For example, the first call yields `bridge.test.0` and the next
/// `bridge.test.1`.
#[fixture]
pub(super) fn unique_logger_name() -> String {
    let suffix = LOGGER_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("bridge.test.{suffix}")
}

/// Set a permissive facade maximum for tests that call the adapter directly.
///
/// `log` starts at `Off`, so without this fixture the adapter returns before
/// the behaviour under test is reachable.
#[fixture]
pub(super) fn log_max_level() {
    static INIT: Once = Once::new();
    INIT.call_once(|| log::set_max_level(log::LevelFilter::Trace));
}
