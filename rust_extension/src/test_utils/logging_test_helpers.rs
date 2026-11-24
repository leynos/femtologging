//! Shared helpers for the logging test suites.
//!
//! The `log` and `tracing` bridges and the Python convenience functions all
//! need the same scaffolding: a logger name no other test in the binary
//! already holds, a facade maximum permissive enough for a direct
//! [`log::Log::log`] call, and a way to run an ignored test in a fresh
//! process. Keeping one copy here stops the four suites drifting apart.
//!
//! Nothing here is compiled outside a test build: the parent
//! [`crate::test_utils`] module is `#[cfg(test)]`.

#[cfg(feature = "log-compat")]
use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};

static LOGGER_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Supply a logger name unique within this test binary.
///
/// One counter is shared by every suite, so no two generated names can
/// collide. For example, the first call yields `test.logger.0` and the next
/// `test.logger.1`.
#[rstest::fixture]
pub(crate) fn unique_logger_name() -> String {
    let suffix = LOGGER_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("test.logger.{suffix}")
}

/// Set a permissive facade maximum for tests that call a bridge directly.
///
/// `log` starts at `Off`, so without this fixture the adapter returns before
/// the behaviour under test is reachable.
#[cfg(feature = "log-compat")]
#[rstest::fixture]
pub(crate) fn log_max_level() {
    static INIT: Once = Once::new();
    INIT.call_once(|| log::set_max_level(log::LevelFilter::Trace));
}

/// Run one ignored test in a fresh copy of the current test executable.
///
/// Cargo supplies the dynamic-library search path when it launches test
/// binaries, and [`crate::test_utils::isolated_child`] preserves that one
/// value so the child can load the same Python shared library. A child that
/// does not exit successfully is reported as an error rather than panicking,
/// so the calling test decides whether that fails its own scenario.
#[cfg(all(
    not(loom),
    any(
        all(feature = "python", feature = "log-compat"),
        all(feature = "python", feature = "tracing-compat")
    )
))]
pub(crate) fn run_isolated_child(test_filter: &str) -> std::io::Result<()> {
    let output = crate::test_utils::isolated_child::run_isolated_test(test_filter)?;
    if output.status.success() {
        return Ok(());
    }
    Err(std::io::Error::other(format!(
        "isolated test {test_filter:?} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )))
}
