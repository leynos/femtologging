use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use std::thread;

use _femtologging_rs::{DefaultFormatter, FemtoFileHandler, FemtoHandlerTrait, FemtoLogRecord};
use rstest::*;
use tempfile::NamedTempFile;

fn read_file(path: &std::path::Path) -> String {
    let mut contents = String::new();
    File::open(path)
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    contents
}

#[rstest]
fn file_handler_writes_to_file() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();
    let handler = FemtoFileHandler::new(&path).unwrap();
    handler.handle(FemtoLogRecord::new("core", "INFO", "hello"));
    drop(handler);
    assert_eq!(read_file(&path), "core [INFO] hello\n");
}

#[rstest]
fn file_handler_multiple_records() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();
    let handler = FemtoFileHandler::with_capacity(&path, DefaultFormatter, 10).unwrap();
    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    handler.handle(FemtoLogRecord::new("core", "WARN", "second"));
    handler.handle(FemtoLogRecord::new("core", "ERROR", "third"));
    drop(handler);
    let output = read_file(&path);
    assert_eq!(
        output,
        "core [INFO] first\ncore [WARN] second\ncore [ERROR] third\n"
    );
}

#[rstest]
fn file_handler_concurrent_usage() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();
    let handler = Arc::new(FemtoFileHandler::new(&path).unwrap());
    let mut handles = vec![];
    for i in 0..10 {
        let h = Arc::clone(&handler);
        handles.push(thread::spawn(move || {
            h.handle(FemtoLogRecord::new("core", "INFO", &format!("msg{}", i)));
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    drop(handler);
    let output = read_file(&path);
    for i in 0..10 {
        assert!(output.contains(&format!("core [INFO] msg{}", i)));
    }
}
