//! Integration tests for log-record metadata and formatting.

use _femtologging_rs::{FemtoLevel, FemtoLogRecord};
use rstest::rstest;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::thread;
use std::time::SystemTime;

// Exercise combinations of level, module path, filename and thread name.

struct ExpectedMetadata<'a> {
    module_path: &'a str,
    filename: &'a str,
    key_values: &'a BTreeMap<String, String>,
    thread_name: Option<&'a str>,
}

#[track_caller]
fn assert_record_content(record: &FemtoLogRecord, expected_level: &str) {
    assert_eq!(
        record.logger(),
        "core",
        "the logger name should be preserved"
    );
    assert_eq!(
        record.level_str(),
        expected_level,
        "the level should use its canonical string"
    );
    assert_eq!(
        record.message(),
        "fail",
        "the log message should be preserved"
    );
}

#[track_caller]
fn assert_record_metadata(record: &FemtoLogRecord, expected: &ExpectedMetadata<'_>) {
    assert!(
        record.metadata().timestamp > SystemTime::UNIX_EPOCH,
        "record timestamp should be after the Unix epoch"
    );
    assert_eq!(
        record.metadata().module_path,
        expected.module_path,
        "module path metadata should be preserved"
    );
    assert_eq!(
        record.metadata().filename,
        expected.filename,
        "filename metadata should be preserved"
    );
    assert_eq!(
        record.metadata().line_number,
        42,
        "source line metadata should be preserved"
    );
    assert_eq!(
        &record.metadata().key_values,
        expected.key_values,
        "key-value metadata should be preserved"
    );
    assert_eq!(
        record.metadata().thread_id,
        thread::current().id(),
        "thread identity metadata should identify the recording thread"
    );
    assert_eq!(
        record.metadata().thread_name.as_deref(),
        expected.thread_name,
        "thread name metadata should be preserved"
    );
}

#[rstest]
fn metadata_sets_fields(
    #[values(FemtoLevel::Info, FemtoLevel::Error)] level: FemtoLevel,
    #[values("", "mod::path")] module_path: &'static str,
    #[values("", "file.rs")] filename: &'static str,
    #[values(None, Some("worker"))] thread_name: Option<&'static str>,
) {
    let expected_thread = thread_name.map(str::to_owned);
    let expected_level = level.as_str();
    let thread_builder = thread::Builder::new();
    let named_thread_builder = if let Some(ref name) = expected_thread {
        thread_builder.name(name.clone())
    } else {
        thread_builder
    };
    named_thread_builder
        .spawn(move || {
            let mut kvs = BTreeMap::new();
            kvs.insert("user".to_owned(), "alice".to_owned());
            let metadata = _femtologging_rs::RecordMetadata {
                module_path: module_path.to_owned(),
                filename: filename.to_owned(),
                line_number: 42,
                key_values: kvs.clone(),
                .._femtologging_rs::RecordMetadata::default()
            };
            let record = FemtoLogRecord::with_metadata("core", level, "fail", metadata);
            let expected_metadata = ExpectedMetadata {
                module_path,
                filename,
                key_values: &kvs,
                thread_name: expected_thread.as_deref(),
            };
            assert_record_content(&record, expected_level);
            assert_record_metadata(&record, &expected_metadata);
        })
        .expect("spawn thread")
        .join()
        .expect("thread joined without panic");
}

/// Test that `level_str()` returns the canonical string for each `FemtoLevel` variant.
#[rstest]
#[case(FemtoLevel::Trace, "TRACE")]
#[case(FemtoLevel::Debug, "DEBUG")]
#[case(FemtoLevel::Info, "INFO")]
#[case(FemtoLevel::Warn, "WARN")]
#[case(FemtoLevel::Error, "ERROR")]
#[case(FemtoLevel::Critical, "CRITICAL")]
fn level_str_returns_canonical_string(#[case] level: FemtoLevel, #[case] expected: &str) {
    let record = FemtoLogRecord::new("test", level, "msg");
    assert_eq!(record.level_str(), expected);
}

/// Test that `Display` for `FemtoLogRecord` includes the level string and message.
///
/// The Display format is `"{level} - {message}"` (logger name is not included).
#[rstest]
#[case(FemtoLevel::Trace, "TRACE")]
#[case(FemtoLevel::Debug, "DEBUG")]
#[case(FemtoLevel::Info, "INFO")]
#[case(FemtoLevel::Warn, "WARN")]
#[case(FemtoLevel::Error, "ERROR")]
#[case(FemtoLevel::Critical, "CRITICAL")]
fn display_includes_level_string(#[case] level: FemtoLevel, #[case] expected_level: &str) {
    let record = FemtoLogRecord::new("mylogger", level, "test message");
    let mut output = String::new();
    write!(&mut output, "{record}").expect("write to string");

    // Display format is "{level} - {message}"
    let expected = format!("{expected_level} - test message");
    assert_eq!(output, expected);
}
