//! Retention behaviour when the log path has no directory component.
//!
//! A bare file name such as `bare.log` has an empty parent, and `read_dir("")`
//! fails on Unix, so retention has to treat that parent as the current
//! directory or silently skip every rollover. The strategy derives that
//! directory from the path it was given, so the only faithful test rotates real
//! files with a bare name — and those land in the process working directory.
//!
//! The working directory is global, so changing it inside the shared test
//! binary would redirect every other test running at the same time. The
//! scenario therefore runs as an ignored test in a child process whose working
//! directory is a temporary directory, and the test left in the ordinary run
//! checks that the child ran it and passed.

use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Duration;
use tempfile::tempdir;

use super::clock::SequenceClock;
use super::rotation_strategy::TimedFileRotationStrategy;
use super::schedule::{TimedRotationSchedule, TimedRotationWhen};
use super::test_helpers::utc_datetime;
use crate::handlers::file::RotationStrategy;

/// Test-name filter that selects the scenario and nothing else.
///
/// The child runs the same binary, so the scenario name is enough to identify
/// it; no crate or module prefix is needed, and the name is distinctive enough
/// that no other test in the binary matches it.
const SCENARIO_FILTER: &str = "bare_name_retention_survives_multiple_rollovers";

/// Run the retention scenario in a child process with its own directory.
///
/// The child inherits the environment deliberately: it is this same test
/// binary and may need the loader path the test harness exported for it, and
/// no inherited setting changes the scenario's outcome. Its working directory
/// is what the scenario must not share with the rest of this binary.
#[test]
fn a_child_directory_runs_the_bare_name_retention_scenario() {
    let directory = tempdir().expect("tempdir must create a temporary directory");
    let executable = std::env::current_exe().expect("the test binary path must resolve");
    let output = Command::new(executable)
        .args([SCENARIO_FILTER, "--ignored"])
        .current_dir(directory.path())
        .output()
        .expect("the child test process must start");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "the scenario must run and pass in its own directory; status: {:?}\n\
         stdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
}

/// Rotate a bare-named log three times with a retention limit of one.
///
/// Ignored in the ordinary run because it writes into the current directory;
/// only the child process above, whose current directory is temporary, runs it.
#[test]
#[ignore = "writes to its working directory; the child-process test above runs it"]
fn bare_name_retention_survives_multiple_rollovers() {
    let schedule = TimedRotationSchedule::new(TimedRotationWhen::Seconds, 1, true, None)
        .expect("seconds schedule must validate");
    let start = utc_datetime!("2026-03-12T00:00:00Z");
    let clock = SequenceClock::new([
        start,
        start,
        start + Duration::seconds(2),
        start + Duration::seconds(4),
    ]);
    let path = PathBuf::from("bare.log");
    let mut strategy = TimedFileRotationStrategy::new_with_clock(path.clone(), schedule, 1, clock);
    let mut writer = BufWriter::new(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("the bare-named log file must open"),
    );

    // Two of these writes follow a rollover, and each rollover prunes. A
    // retention pass that cannot find the directory reports an error here
    // rather than leaving the stale backup behind, so the success of every
    // call is part of the regression.
    for record in ["first", "second", "third"] {
        RotationStrategy::before_write(&mut strategy, &mut writer, record)
            .expect("rotation must not fail for a path without a directory component");
        writeln!(writer, "{record}").expect("the record must be written");
        writer.flush().expect("the record must reach the file");
    }

    let oldest = Path::new("bare.log.2026-03-12_00-00-00");
    let retained = Path::new("bare.log.2026-03-12_00-00-02");
    assert!(!oldest.exists(), "the oldest backup must be pruned");
    assert!(retained.exists(), "the most recent backup must be retained");

    let backups = fs::read_dir(".")
        .expect("the working directory must be readable")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("bare.log."))
        .count();
    assert_eq!(
        backups, 1,
        "a retention limit of one must leave exactly one backup"
    );
}
