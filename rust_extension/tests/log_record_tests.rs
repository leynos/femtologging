use _femtologging_rs::FemtoLogRecord;
use std::collections::BTreeMap;
use std::thread;
use std::time::SystemTime;

#[test]
fn new_populates_metadata() {
    let before = SystemTime::now();
    let record = FemtoLogRecord::new("core", "INFO", "hello");
    assert_eq!(record.logger, "core");
    assert_eq!(record.level, "INFO");
    assert_eq!(record.message, "hello");
    assert!(record.timestamp >= before && record.timestamp <= SystemTime::now());
    assert_eq!(record.module_path, "");
    assert_eq!(record.filename, "");
    assert_eq!(record.line_number, 0);
    assert_eq!(record.thread_id, thread::current().id());
    assert_eq!(
        record.thread_name,
        thread::current().name().map(|s| s.to_string())
    );
    assert!(record.key_values.is_empty());
}

#[test]
fn with_metadata_sets_fields() {
    let mut kvs = BTreeMap::new();
    kvs.insert("user".to_string(), "alice".to_string());
    let record = FemtoLogRecord::with_metadata(
        "core",
        "ERROR",
        "fail",
        "mod::path",
        "file.rs",
        42,
        kvs.clone(),
    );
    assert_eq!(record.module_path, "mod::path");
    assert_eq!(record.filename, "file.rs");
    assert_eq!(record.line_number, 42);
    assert_eq!(record.key_values, kvs);
}
