use std::io::{self, Write};
use std::sync::Barrier;
use std::thread;
use std::time::{Duration, Instant};

use _femtologging_rs::{DefaultFormatter, FemtoHandlerTrait, FemtoLogRecord, FemtoStreamHandler};
use log;
use logtest;
use rstest::*;
use serial_test::serial;

mod test_utils;
use std::sync::{Arc, Mutex};
use test_utils::fixtures::{handler_tuple, handler_tuple_custom};
use test_utils::shared_buffer::std::read_output;
use test_utils::std::SharedBuf;

#[derive(Clone)]
struct BlockingBuf {
    buf: Arc<Mutex<Vec<u8>>>,
    barrier: Arc<Barrier>,
}

impl Write for BlockingBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        // Block until the test thread releases the barrier
        self.barrier.wait();
        self.buf.lock().unwrap().flush()
    }
}

#[rstest]
fn stream_handler_writes_to_buffer(
    #[from(handler_tuple)] (buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    handler.handle(FemtoLogRecord::new("core", "INFO", "hello"));
    drop(handler); // ensure thread completes

    assert_eq!(read_output(&buffer), "core [INFO] hello\n");
}

#[rstest]
fn stream_handler_multiple_records(
    #[from(handler_tuple)] (buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    handler.handle(FemtoLogRecord::new("core", "WARN", "second"));
    handler.handle(FemtoLogRecord::new("core", "ERROR", "third"));
    drop(handler);

    let output = read_output(&buffer);
    assert_eq!(
        output,
        "core [INFO] first\ncore [WARN] second\ncore [ERROR] third\n"
    );
}

#[rstest]
fn stream_handler_flush(
    #[from(handler_tuple)] (buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    handler.handle(FemtoLogRecord::new("core", "INFO", "one"));
    assert!(handler.flush());
    handler.handle(FemtoLogRecord::new("core", "INFO", "two"));
    drop(handler);

    assert_eq!(read_output(&buffer), "core [INFO] one\ncore [INFO] two\n");
}

#[rstest]
fn stream_handler_close_flushes_pending(
    #[from(handler_tuple)] (buffer, mut handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    handler.handle(FemtoLogRecord::new("core", "INFO", "close"));
    handler.close();

    assert_eq!(read_output(&buffer), "core [INFO] close\n");
}

#[rstest]
fn stream_handler_flush_after_close(
    #[from(handler_tuple)] (_buffer, mut handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    handler.close();
    assert!(!handler.flush());
}

#[rstest]
fn stream_handler_concurrent_usage(
    #[from(handler_tuple)] (buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    let handler = Arc::new(handler);

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

    let output = read_output(&buffer);
    for i in 0..10 {
        assert!(output.contains(&format!("core [INFO] msg{}", i)));
    }
}

#[rstest]
fn stream_handler_trait_object_usage(
    #[from(handler_tuple)] (buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    let handler: Box<dyn FemtoHandlerTrait> = Box::new(handler);
    handler.handle(FemtoLogRecord::new("core", "INFO", "trait"));
    drop(handler);

    assert_eq!(read_output(&buffer), "core [INFO] trait\n");
}

#[rstest]
fn stream_handler_poisoned_mutex(
    #[from(handler_tuple)] (buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    // Poison the mutex by panicking while holding the lock
    let test_buffer = Arc::clone(&buffer);
    {
        let b = Arc::clone(&buffer);
        let _ = std::panic::catch_unwind(move || {
            let _guard = b.lock().unwrap();
            panic!("poison");
        });
    }

    handler.handle(FemtoLogRecord::new("core", "INFO", "ok"));
    drop(handler);

    // The buffer should remain poisoned; handler must not panic
    assert!(
        test_buffer.lock().is_err(),
        "Buffer mutex should remain poisoned",
    );
}

#[rstest]
/// Ensure dropping a handler with a slow writer doesn't block
/// indefinitely. The worker thread should exit after the one
/// second timeout even if the stream flush takes longer. The test
/// allows a 500ms buffer to accommodate scheduling jitter.
fn stream_handler_drop_timeout() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let barrier = Arc::new(Barrier::new(2));
    let handler = FemtoStreamHandler::new(
        BlockingBuf {
            buf: Arc::clone(&buffer),
            barrier: Arc::clone(&barrier),
        },
        DefaultFormatter,
    );
    handler.handle(FemtoLogRecord::new("core", "INFO", "slow"));
    let start = Instant::now();
    drop(handler);
    assert!(start.elapsed() < Duration::from_millis(1500));
    // The extra half second gives the test leeway for scheduler jitter
    // while still proving the drop doesn't hang indefinitely.
    // Allow the worker thread to finish
    barrier.wait();
}

#[rstest]
#[serial]
#[ignore]
fn stream_handler_reports_dropped_records() {
    let logger = logtest::start();
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let handler = FemtoStreamHandler::with_capacity_timeout(
        SharedBuf::new(Arc::clone(&buffer)),
        DefaultFormatter,
        1,
        Duration::from_millis(50),
    );

    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    handler.handle(FemtoLogRecord::new("core", "INFO", "second"));
    assert!(handler.flush());

    let warnings: Vec<_> = logger
        .into_iter()
        .filter(|r| r.level() == log::Level::Warn)
        .collect();
    assert!(warnings
        .iter()
        .any(|r| r.args().to_string().contains("1 log records dropped")));
}

#[rstest]
#[serial]
#[ignore]
fn stream_handler_rate_limits_warnings(
    #[from(handler_tuple_custom)]
    #[with(Duration::from_millis(50))]
    (_buffer, handler): (Arc<Mutex<Vec<u8>>>, FemtoStreamHandler),
) {
    let logger = logtest::start();
    // First drop triggers a warning
    handler.handle(FemtoLogRecord::new("core", "INFO", "first"));
    handler.handle(FemtoLogRecord::new("core", "INFO", "second"));
    assert!(handler.flush());

    // Second drop within interval should be suppressed
    handler.handle(FemtoLogRecord::new("core", "INFO", "third"));
    handler.handle(FemtoLogRecord::new("core", "INFO", "fourth"));
    assert!(handler.flush());

    // Wait for interval to elapse then drop again
    std::thread::sleep(Duration::from_millis(60));
    handler.handle(FemtoLogRecord::new("core", "INFO", "fifth"));
    handler.handle(FemtoLogRecord::new("core", "INFO", "sixth"));
    assert!(handler.flush());

    let warnings: Vec<_> = logger
        .into_iter()
        .filter(|r| r.level() == log::Level::Warn)
        .collect();
    assert_eq!(warnings.len(), 2);
}
