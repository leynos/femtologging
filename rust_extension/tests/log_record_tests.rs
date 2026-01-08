use _femtologging_rs::{FemtoLevel, FemtoLogRecord};
use rstest::rstest;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::thread;
use std::time::SystemTime;

// Exercise combinations of level, module path, filename and thread name.

#[rstest]
fn metadata_sets_fields(
    #[values(FemtoLevel::Info, FemtoLevel::Error)] level: FemtoLevel,
    #[values("", "mod::path")] module_path: &'static str,
    #[values("", "file.rs")] filename: &'static str,
    #[values(None, Some("worker"))] thread_name: Option<&'static str>,
) {
    let expected_thread = thread_name.map(str::to_string);
    let expected_level = level.as_str();
    let builder = thread::Builder::new();
    let builder = if let Some(ref name) = expected_thread {
        builder.name(name.clone())
    } else {
        builder
    };
    builder
        .spawn(move || {
            let mut kvs = BTreeMap::new();
            kvs.insert("user".to_string(), "alice".to_string());
            let metadata = _femtologging_rs::RecordMetadata {
                module_path: module_path.to_string(),
                filename: filename.to_string(),
                line_number: 42,
                key_values: kvs.clone(),
                .._femtologging_rs::RecordMetadata::default()
            };
            let record = FemtoLogRecord::with_metadata("core", level, "fail", metadata);
            assert_eq!(record.logger(), "core");
            assert_eq!(record.level_str(), expected_level);
            assert_eq!(record.message(), "fail");
            assert!(record.metadata().timestamp > SystemTime::UNIX_EPOCH);
            assert_eq!(record.metadata().module_path, module_path);
            assert_eq!(record.metadata().filename, filename);
            assert_eq!(record.metadata().line_number, 42);
            assert_eq!(record.metadata().key_values, kvs);
            assert_eq!(record.metadata().thread_id, thread::current().id());
            assert_eq!(
                record.metadata().thread_name.as_deref(),
                expected_thread.as_deref()
            );
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
    write!(&mut output, "{}", record).expect("write to string");

    // Display format is "{level} - {message}"
    let expected = format!("{} - test message", expected_level);
    assert_eq!(output, expected);
}
