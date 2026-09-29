//! Concurrency test for FemtoFileHandler flush behaviour.
//!
//! Uses loom to explore potential interleavings when multiple threads
//! call `flush()` simultaneously while writing records.
//!
//! The handler writes into a Loom-instrumented buffer rather than a file, so
//! the record count the model asserts is an effect the model can observe. Real
//! file output is covered by the ordinary file-handler integration tests.
//!
//! Two writers, the model's own thread and the handler's worker make four
//! threads. Three writers ran for over thirty minutes without a verdict at
//! the lane's preemption bound, so the user chose two on 2026-09-25; the
//! ordinary file-handler integration tests cover wider fan-in.

use loom::sync::{Arc, Mutex};
use loom::thread;

use _femtologging_rs::{
    DefaultFormatter, FemtoFileHandler, FemtoLevel, FemtoLogRecord, OverflowPolicy, TestConfig,
};

use crate::handle_expect::HandleExpect;
use crate::shared_buffer::loom::SharedBuf as LoomBuf;
use crate::shared_buffer::loom::read_output;

/// The number of threads writing and flushing concurrently.
const WRITERS: usize = 2;

#[test]
fn loom_file_handler_flush_concurrent() {
    loom::model(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let mut config = TestConfig::new(LoomBuf::new(Arc::clone(&buffer)), DefaultFormatter);
        config.capacity = 8;
        config.flush_interval = 1;
        config.overflow_policy = OverflowPolicy::Drop;
        let handler = Arc::new(FemtoFileHandler::with_writer_for_test(config));

        let writers: Vec<_> = (0..WRITERS)
            .map(|writer| {
                let h = Arc::clone(&handler);
                let written = Arc::clone(&buffer);
                thread::spawn(move || {
                    let message = format!("msg{writer}");
                    h.expect_handle(FemtoLogRecord::new("core", FemtoLevel::Info, &message));
                    assert!(h.flush(), "a flush after a write must be acknowledged");
                    assert!(
                        read_output(&written).contains(&format!("core [INFO] {message}")),
                        "an acknowledged flush must follow the writer's own record"
                    );
                })
            })
            .collect();

        for writer in writers {
            writer.join().expect("thread panicked");
        }

        drop(handler);
        assert_eq!(read_output(&buffer).lines().count(), WRITERS);
    });
}
