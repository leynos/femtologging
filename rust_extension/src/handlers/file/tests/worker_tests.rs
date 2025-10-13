#![cfg(all(test, feature = "python"))]
//! Tests focusing on the worker thread configuration and error handling paths.

use super::super::test_support::{install_test_logger, take_logged_messages};
use super::super::*;
use super::test_support::SharedBuf;
use log::Level;
use serial_test::serial;
use std::io::{self, ErrorKind, Seek, SeekFrom, Write};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

#[test]
fn worker_config_from_handlerconfig_copies_values() {
    let cfg = HandlerConfig {
        capacity: 42,
        flush_interval: 7,
        overflow_policy: OverflowPolicy::Drop,
    };
    let worker = WorkerConfig::from(&cfg);
    assert_eq!(worker.capacity, 42);
    assert_eq!(worker.flush_interval, 7);
    assert!(worker.start_barrier.is_none());
}

#[test]
#[serial]
fn worker_writes_record_when_rotation_fails() {
    struct FailingRotation;

    impl RotationStrategy<SharedBuf> for FailingRotation {
        fn before_write(&mut self, _writer: &mut SharedBuf, _formatted: &str) -> io::Result<bool> {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "failing rotation for test",
            ))
        }
    }

    let buffer = SharedBuf::default();
    let writer = buffer.clone();
    let handler_cfg = HandlerConfig {
        capacity: 1,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Block,
    };
    let options = BuilderOptions::<SharedBuf, FailingRotation>::new(FailingRotation, None);
    install_test_logger();
    let mut handler =
        FemtoFileHandler::build_from_worker(writer, DefaultFormatter, handler_cfg, options);

    handler
        .handle(FemtoLogRecord::new(
            "core",
            "INFO",
            "after rotation failure",
        ))
        .expect("record queued after rotation warning");
    assert!(
        handler.flush(),
        "flush should succeed even if rotation reported an error",
    );
    handler.close();

    let logs = take_logged_messages();
    assert!(
        logs.iter().any(|record| {
            record.level == Level::Error
                && record
                    .message
                    .contains("FemtoFileHandler rotation error; writing record without rotating")
        }),
        "rotation error should be logged",
    );

    assert_eq!(buffer.contents(), "core [INFO] after rotation failure\n");
}

#[test]
fn femto_file_handler_worker_thread_failure() {
    #[derive(Clone)]
    struct BlockingWriter {
        buf: Arc<Mutex<Vec<u8>>>,
        barrier: Arc<Barrier>,
    }

    impl Write for BlockingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.buf.lock().unwrap().write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.barrier.wait();
            self.buf.lock().unwrap().flush()
        }
    }

    impl Seek for BlockingWriter {
        fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
            Err(io::Error::new(
                ErrorKind::Unsupported,
                "seek unsupported for BlockingWriter",
            ))
        }
    }

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let barrier = Arc::new(Barrier::new(2));
    let mut cfg = TestConfig::new(
        BlockingWriter {
            buf: Arc::clone(&buffer),
            barrier: Arc::clone(&barrier),
        },
        DefaultFormatter,
    );
    cfg.capacity = 1;
    cfg.flush_interval = 1;
    let handler = FemtoFileHandler::with_writer_for_test(cfg);
    handler
        .handle(FemtoLogRecord::new("core", "INFO", "slow"))
        .expect("record queued");
    let start = Instant::now();
    drop(handler);
    assert!(start.elapsed() < Duration::from_millis(1500));
    barrier.wait();
}

#[test]
#[serial]
fn worker_logs_repeated_rotation_failures() {
    struct FlakyRotation {
        failures: usize,
        call: usize,
    }

    impl RotationStrategy<SharedBuf> for FlakyRotation {
        fn before_write(&mut self, _writer: &mut SharedBuf, _formatted: &str) -> io::Result<bool> {
            let call = self.call;
            self.call += 1;
            if call < self.failures {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("flaky rotation failure #{call}"),
                ))
            } else {
                Ok(false)
            }
        }
    }

    let buffer = SharedBuf::default();
    let writer = buffer.clone();
    let handler_cfg = HandlerConfig {
        capacity: 1,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Block,
    };
    let options = BuilderOptions::<SharedBuf, FlakyRotation>::new(
        FlakyRotation {
            failures: 3,
            call: 0,
        },
        None,
    );

    install_test_logger();
    let mut handler =
        FemtoFileHandler::build_from_worker(writer, DefaultFormatter, handler_cfg, options);

    for i in 0..5 {
        handler
            .handle(FemtoLogRecord::new(
                "core",
                "INFO",
                &format!("repeated-{i}"),
            ))
            .expect("record queued despite rotation failure");
    }

    assert!(
        handler.flush(),
        "flush should succeed after repeated failures"
    );
    handler.close();

    let logs = take_logged_messages();
    let errors: Vec<_> = logs
        .iter()
        .filter(|record| {
            record.level == Level::Error
                && record
                    .message
                    .contains("FemtoFileHandler rotation error; writing record without rotating")
        })
        .collect();
    assert_eq!(errors.len(), 3, "each failed rotation should be logged");
    for idx in 0..3 {
        assert!(
            errors.iter().any(|record| record
                .message
                .contains(&format!("flaky rotation failure #{idx}"))),
            "log should include failure #{idx}",
        );
    }

    let expected = (0..5)
        .map(|i| format!("core [INFO] repeated-{i}\n"))
        .collect::<String>();
    assert_eq!(buffer.contents(), expected);
}
