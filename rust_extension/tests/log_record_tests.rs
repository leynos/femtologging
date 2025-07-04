use _femtologging_rs::FemtoLogRecord;
use std::collections::BTreeMap;
use std::thread;
use std::time::{Duration, SystemTime};

#[test]
fn new_populates_metadata() {
    let before = SystemTime::now();
    let record = FemtoLogRecord::new("core", "INFO", "hello");
    assert_eq!(record.logger, "core");
    assert_eq!(record.level, "INFO");
    assert_eq!(record.message, "hello");
    let now = SystemTime::now();
    assert!(
        record.metadata.timestamp <= now,
        "timestamp is in the future"
    );
    assert!(
        record.metadata.timestamp >= before - Duration::from_secs(5),
        "timestamp is too far in the past"
    );
    assert_eq!(record.metadata.module_path, "");
    assert_eq!(record.metadata.filename, "");
    assert_eq!(record.metadata.line_number, 0);
    assert_eq!(record.metadata.thread_id, thread::current().id());
    assert_eq!(
        record.metadata.thread_name,
        thread::current().name().map(|s| s.to_string())
    );
    assert!(record.metadata.key_values.is_empty());
}

#[test]
fn with_metadata_sets_fields() {
    let mut kvs = BTreeMap::new();
    kvs.insert("user".to_string(), "alice".to_string());
    let metadata = _femtologging_rs::RecordMetadata {
        module_path: "mod::path".to_string(),
        filename: "file.rs".to_string(),
        line_number: 42,
        key_values: kvs.clone(),
        .._femtologging_rs::RecordMetadata::default()
    };
    let record = FemtoLogRecord::with_metadata("core", "ERROR", "fail", metadata.clone());
    assert_eq!(record.logger, "core");
    assert_eq!(record.level, "ERROR");
    assert_eq!(record.message, "fail");
    assert!(record.metadata.timestamp > SystemTime::UNIX_EPOCH);
    assert_eq!(record.metadata.module_path, "mod::path");
    assert_eq!(record.metadata.filename, "file.rs");
    assert_eq!(record.metadata.line_number, 42);
    assert_eq!(record.metadata.key_values, kvs);
    assert_eq!(record.metadata.thread_id, thread::current().id());
    assert_eq!(
        record.metadata.thread_name,
        thread::current().name().map(|s| s.to_string())
    );
}
