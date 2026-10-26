//! Helpers for running Rust tests in a clean child process.

use std::io;
use std::process::{Command, Output};

use std::ffi::OsString;

/// Run one ignored test in a fresh copy of the current test executable.
///
/// Cargo supplies the dynamic-library search path when it launches test
/// binaries. Preserve that one value so the child can load the same Python
/// shared library while keeping unrelated ambient configuration out.
pub(crate) fn run_isolated_test(test_filter: &str) -> io::Result<Output> {
    let executable = std::env::current_exe()?;
    let mut command = Command::new(executable);
    command.env_clear();
    if let Some((name, value)) = cargo_loader_search_path() {
        command.env(name, value);
    }
    command.args(["--ignored", test_filter, "--nocapture"]);
    command.output()
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
