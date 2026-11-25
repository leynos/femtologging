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

#[cfg(all(
    not(loom),
    any(
        all(feature = "python", feature = "log-compat"),
        all(feature = "python", feature = "tracing-compat")
    )
))]
use std::process::Output;

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
///
/// A child whose filter selects no test still exits successfully, so the
/// harness's `1 passed` summary is required as well as the status. Without it a
/// renamed or filtered-out child would report a passing parent while running
/// nothing. The count is read as a whole word, so `11 passed` does not stand in
/// for the single test the filter names.
#[cfg(all(
    not(loom),
    any(
        all(feature = "python", feature = "log-compat"),
        all(feature = "python", feature = "tracing-compat")
    )
))]
pub(crate) fn run_isolated_child(test_filter: &str) -> std::io::Result<()> {
    let output = crate::test_utils::isolated_child::run_isolated_test(test_filter)?;
    judge_child_output(test_filter, &output)
}

/// Return `Ok` only when a child ran and passed the test its filter named.
///
/// The rule is separated from process launching so it can be exercised with
/// synthetic output: a child that exits successfully but reports `0 passed`,
/// or `2 passed`, did not run the one test the caller meant to isolate.
///
/// The gate matches its only caller's. `Output` itself is only imported under
/// that feature set, and the rule has no caller elsewhere.
#[cfg(all(
    not(loom),
    any(
        all(feature = "python", feature = "log-compat"),
        all(feature = "python", feature = "tracing-compat")
    )
))]
fn judge_child_output(test_filter: &str, output: &Output) -> std::io::Result<()> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status.success() && reports_one_passed(&stdout) {
        return Ok(());
    }
    Err(std::io::Error::other(format!(
        "isolated test {test_filter:?} must run and pass; status: {:?}\nstdout:\n{stdout}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    )))
}

/// Return whether a child's stdout carries the harness's `1 passed` summary.
///
/// A substring search for `1 passed` would also accept `11 passed`, so the
/// count is read as a whole word: the line must read exactly `test result: ok.
/// 1 passed`, optionally followed by the harness's `; N filtered out` and the
/// timing clause.
///
/// Gated with its only caller; without that caller the function would be dead
/// code in a Loom build.
#[cfg(all(
    not(loom),
    any(
        all(feature = "python", feature = "log-compat"),
        all(feature = "python", feature = "tracing-compat")
    )
))]
fn reports_one_passed(stdout: &str) -> bool {
    stdout.lines().any(|line| {
        let Some(rest) = line.trim().strip_prefix("test result: ok. ") else {
            return false;
        };
        rest.strip_prefix("1 passed").is_some_and(|tail| {
            tail.is_empty() || tail.starts_with(';') || tail.starts_with(" in ")
        })
    })
}

// The rule itself is platform-neutral, but the test builds a synthetic
// `ExitStatus`, which only Unix exposes a constructor for.
#[cfg(all(
    unix,
    not(loom),
    any(
        all(feature = "python", feature = "log-compat"),
        all(feature = "python", feature = "tracing-compat")
    )
))]
#[cfg(test)]
mod tests {
    //! The child-output rule, exercised without launching a process.

    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    use super::judge_child_output;

    /// Build an `Output` with the given exit code and stdout.
    fn output(code: i32, stdout: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    /// The summary line the Rust harness prints for one passing test.
    const ONE_PASSED: &str =
        "test chosen ... ok\n\ntest result: ok. 1 passed; 3 filtered out; finished in 0.01s";

    #[test]
    fn a_child_reporting_one_passed_is_accepted() {
        assert!(judge_child_output("chosen", &output(0, ONE_PASSED)).is_ok());
    }

    #[test]
    fn the_summary_count_is_read_as_a_whole_word() {
        let eleven = "test result: ok. 11 passed; 0 filtered out; finished in 0.4s";
        assert!(
            judge_child_output("chosen", &output(0, eleven)).is_err(),
            "`11 passed` must not satisfy a rule that asks for one test"
        );
    }

    #[test]
    fn a_summary_without_a_timing_clause_is_accepted() {
        assert!(
            judge_child_output("chosen", &output(0, "test result: ok. 1 passed")).is_ok(),
            "the timing clause is optional in the harness summary"
        );
    }

    #[test]
    fn a_successful_child_that_ran_nothing_is_refused() {
        let ran_nothing = "test result: ok. 0 passed; 3 filtered out; finished in 0.00s";
        let error = judge_child_output("chosen", &output(0, ran_nothing))
            .expect_err("a filter that selects no test must not report success");
        assert!(error.to_string().contains("must run and pass"), "{error}");
    }

    #[test]
    fn a_successful_child_that_ran_two_tests_is_refused() {
        let two = "test result: ok. 2 passed; 0 filtered out; finished in 0.01s";
        assert!(
            judge_child_output("chosen", &output(0, two)).is_err(),
            "a filter that admits a sibling test must not report success"
        );
    }

    #[test]
    fn a_failing_child_is_refused_even_when_it_reports_a_pass() {
        assert!(
            judge_child_output("chosen", &output(101, ONE_PASSED)).is_err(),
            "a non-zero exit status must not be reported as success"
        );
    }
}
