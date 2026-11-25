//! Helpers for running Rust tests in a clean child process.
//!
//! `run_isolated_test` re-executes the current test binary with `--ignored`,
//! the caller's filter and `--nocapture`, so a test that needs a pristine
//! process can run alone; `run_isolated_test_in` adds a working directory for
//! tests that resolve relative paths. `isolated_command` clears the
//! environment and forwards only the loader search path from
//! `cargo_loader_search_path`, because Cargo supplies the variable that lets
//! the child load the same Python shared library.

use std::io;
use std::path::Path;
use std::process::{Command, Output};

use std::ffi::OsString;

/// Run one ignored test in a fresh copy of the current test executable.
///
/// Cargo supplies the dynamic-library search path when it launches test
/// binaries. Preserve that one value so the child can load the same Python
/// shared library while keeping unrelated ambient configuration out.
///
/// Only the Python bridge saturation tests call this, so it is absent from a
/// build with neither bridge feature; the retention helper below is
/// feature-independent.
#[cfg(any(
    all(feature = "python", feature = "log-compat"),
    all(feature = "python", feature = "tracing-compat")
))]
pub(crate) fn run_isolated_test(test_filter: &str) -> io::Result<Output> {
    isolated_command(test_filter)?.output()
}

/// Run one ignored test in a fresh copy of the current test executable, with
/// `working_directory` as its working directory.
///
/// A test that resolves a bare file name against the current directory needs a
/// directory of its own, because the working directory is global to the test
/// binary. Everything else, including the cleared environment, matches
/// `run_isolated_test`.
pub(crate) fn run_isolated_test_in(
    test_filter: &str,
    working_directory: &Path,
) -> io::Result<Output> {
    let mut command = isolated_command(test_filter)?;
    command.current_dir(working_directory);
    command.output()
}

/// Build a child test command with the ambient environment cleared.
///
/// Only Cargo's loader search path is forwarded, so a variable the parent
/// happens to carry cannot change the child's outcome.
fn isolated_command(test_filter: &str) -> io::Result<Command> {
    let executable = std::env::current_exe()?;
    let mut command = Command::new(executable);
    command.env_clear();
    if let Some((name, value)) = cargo_loader_search_path() {
        command.env(name, value);
    }
    command.args(["--ignored", test_filter, "--nocapture"]);
    Ok(command)
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "composition root: Cargo's LD_LIBRARY_PATH is required to load Python in the isolated test child"
)]
fn cargo_loader_search_path() -> Option<(&'static str, OsString)> {
    std::env::var_os("LD_LIBRARY_PATH").map(|value| ("LD_LIBRARY_PATH", value))
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_methods,
    reason = "composition root: Cargo's dynamic-library search path is required to load Python in the isolated test child"
)]
fn cargo_loader_search_path() -> Option<(&'static str, OsString)> {
    std::env::var_os("DYLD_FALLBACK_LIBRARY_PATH")
        .map(|value| ("DYLD_FALLBACK_LIBRARY_PATH", value))
}

#[cfg(windows)]
#[expect(
    clippy::disallowed_methods,
    reason = "composition root: Cargo's PATH is required to load Python in the isolated test child"
)]
fn cargo_loader_search_path() -> Option<(&'static str, OsString)> {
    std::env::var_os("PATH").map(|value| ("PATH", value))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn cargo_loader_search_path() -> Option<(&'static str, OsString)> {
    None
}
