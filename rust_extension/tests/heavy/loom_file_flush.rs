//! Concurrency test for FemtoFileHandler flush behaviour.
//!
//! Uses loom to explore potential interleavings when multiple threads
//! call `flush()` simultaneously while writing records.

use loom::sync::Arc;
use loom::thread;
use tempfile::NamedTempFile;

use _femtologging_rs::{
    DefaultFormatter, FemtoFileHandler, FemtoLogRecord, HandlerConfig, OverflowPolicy,
};

#[test]
#[ignore]
fn loom_file_handler_flush_concurrent() {
    loom::model(|| {
        let tmp = NamedTempFile::new().expect("create temp file");
        let path = tmp.path().to_path_buf();
        let cfg = HandlerConfig {
            capacity: 8,
            flush_interval: 1,
            overflow_policy: OverflowPolicy::Drop,
        };
        let handler = Arc::new(
            FemtoFileHandler::with_capacity_flush_policy(&path, DefaultFormatter, cfg)
                .expect("create handler"),
        );

        let mut threads = vec![];
        for _ in 0..5 {
            let h = Arc::clone(&handler);
            threads.push(thread::spawn(move || {
                h.handle(FemtoLogRecord::new("core", "INFO", "msg"));
                assert!(h.flush());
            }));
        }

        for t in threads {
            t.join().expect("thread panicked");
        }

        drop(handler);
        let output = std::fs::read_to_string(&path).expect("read file");
        assert_eq!(output.lines().count(), 5);
    });
}
