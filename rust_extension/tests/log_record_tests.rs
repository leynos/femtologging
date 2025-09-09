use _femtologging_rs::FemtoLogRecord;
use rstest::rstest;
use std::collections::BTreeMap;
use std::thread;
use std::time::SystemTime;

// Exercise combinations of level, module path, filename and thread name.

#[rstest]
fn metadata_sets_fields(
    #[values("INFO", "ERROR")] level: &'static str,
    #[values("", "mod::path")] module_path: &'static str,
    #[values("", "file.rs")] filename: &'static str,
    #[values(None, Some("worker"))] thread_name: Option<&'static str>,
) {
    let expected_thread = thread_name.map(str::to_string);
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
            assert_eq!(record.logger, "core");
            assert_eq!(record.level, level);
            assert_eq!(record.message, "fail");
            assert!(record.metadata.timestamp > SystemTime::UNIX_EPOCH);
            assert_eq!(record.metadata.module_path, module_path);
            assert_eq!(record.metadata.filename, filename);
            assert_eq!(record.metadata.line_number, 42);
            assert_eq!(record.metadata.key_values, kvs);
            assert_eq!(record.metadata.thread_id, thread::current().id());
            assert_eq!(
                record.metadata.thread_name.as_deref(),
                expected_thread.as_deref()
            );
        })
        .expect("spawn thread")
        .join()
        .unwrap();
}
