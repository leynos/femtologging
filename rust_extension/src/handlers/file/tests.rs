use super::*;
use std::io::Write;

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
fn build_from_worker_wires_handler_components() {
    #[derive(Clone)]
    struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for Buf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("failed to acquire buffer lock for write")
                .write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.0
                .lock()
                .expect("failed to acquire buffer lock for flush")
                .flush()
        }
    }

    let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer = Buf(std::sync::Arc::clone(&buffer));
    let worker_cfg = WorkerConfig {
        capacity: 1,
        flush_interval: 1,
        start_barrier: None,
    };
    let policy = OverflowPolicy::Block;
    let mut handler =
        FemtoFileHandler::build_from_worker(writer, DefaultFormatter, worker_cfg, policy);

    assert!(handler.tx.is_some());
    assert!(handler.handle.is_some());
    assert_eq!(handler.overflow_policy, policy);

    let tx = handler.tx.take().expect("tx missing");
    let done_rx = handler.done_rx.clone();
    let handle = handler.handle.take().expect("handle missing");

    tx.send(FileCommand::Record(FemtoLogRecord::new(
        "core", "INFO", "test",
    )))
    .expect("send");
    drop(tx);

    assert!(done_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .is_ok());
    handle.join().expect("worker thread");

    let output = String::from_utf8(
        buffer
            .lock()
            .expect("failed to acquire buffer lock for read")
            .clone(),
    )
    .expect("buffer contained invalid UTF-8");
    assert_eq!(output, "core [INFO] test\n");
}
