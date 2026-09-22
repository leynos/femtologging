//! One capture logger per process, drained before each use.
//!
//! `log::set_logger` succeeds once per process and returns an error for every
//! later call, which `logtest` unwraps. Two tests that each called
//! `logtest::start()` therefore could not both run in one binary: whichever
//! ran second panicked, and only which one lost varied.
//!
//! Installing once is not enough on its own. `logtest::Logger` holds no state;
//! every captured record lives in a queue inside `logtest` that outlives the
//! test that produced it. A test asserting on an exact count would read the
//! previous test's records as its own, so the queue is drained under the same
//! lock that hands out the logger, making the drain and the use one operation
//! rather than two that an ordering could separate.
//!
//! This is the shape `install_test_logger` already uses in
//! `src/handlers/file/test_support.rs`: install once, tolerate a prior
//! installation, then clear. The same rule in a second place, because an
//! integration test cannot reach that helper, which is `#[cfg(test)]` and
//! crate-private.

use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

static LOGGER: OnceLock<Mutex<logtest::Logger>> = OnceLock::new();

/// Return the process's capture logger, holding no records from earlier tests.
///
/// The guard is held for as long as the caller needs the logger, so a second
/// test cannot drain records out from under the first. Poisoning is recovered
/// rather than propagated: a panic elsewhere has already failed that test, and
/// turning it into a second panic here would hide the first.
///
/// # Panics
///
/// Never, in the sense that matters: the only failure `logtest::start()` can
/// report is a logger already installed, and `OnceLock` guarantees it is
/// called at most once for the life of the process.
pub fn captured_log() -> MutexGuard<'static, logtest::Logger> {
    let mut guard = LOGGER
        .get_or_init(|| Mutex::new(logtest::start()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    while guard.pop().is_some() {}
    guard
}
