#![cfg(all(test, feature = "python"))]
//! Unit tests for the file handler implementation.
//!
//! These tests verify the wiring between configuration and worker threads as
//! well as basic flushing behaviour. Additional submodules cover overflow,
//! worker error handling, and lifecycle-specific scenarios.

use super::worker::FileCommand;
pub(super) use super::worker::WorkerConfig;
pub(super) use super::{
    BuilderOptions, DefaultFormatter, FemtoFileHandler, FemtoLogRecord, HandlerConfig,
    OverflowPolicy, RotationStrategy, TestConfig,
};
pub(super) use crate::handler::HandlerError;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use self::test_support::{CountingRotation, FlagRotation, SharedBuf};

#[test]
fn builder_options_default_provides_noop_rotation() {
    let mut options: BuilderOptions<Cursor<Vec<u8>>> = BuilderOptions::default();
    assert!(options.start_barrier.is_none());
    let mut writer = Cursor::new(Vec::new());
    assert!(options.rotation.before_write(&mut writer, "entry").is_ok());
}

#[test]
fn builder_options_new_stores_rotation_and_barrier() {
    let flag = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(1));
    let mut options = BuilderOptions::<Cursor<Vec<u8>>, FlagRotation>::new(
        FlagRotation::new(Arc::clone(&flag)),
        Some(Arc::clone(&barrier)),
    );

    let stored = options.start_barrier.take().expect("missing barrier");
    assert!(Arc::ptr_eq(&stored, &barrier));

    let mut writer = Cursor::new(Vec::new());
    options
        .rotation
        .before_write(&mut writer, "check")
        .expect("rotation should succeed");
    assert!(flag.load(Ordering::SeqCst));
}

#[test]
fn build_from_worker_invokes_rotation_strategy() {
    let buffer = SharedBuf::default();
    let writer = buffer.clone();
    let handler_cfg = HandlerConfig {
        capacity: 4,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Block,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let rotation = CountingRotation::new(Arc::clone(&calls));
    let mut handler = FemtoFileHandler::build_from_worker(
        writer,
        DefaultFormatter,
        handler_cfg,
        BuilderOptions::<SharedBuf, _>::new(rotation, None),
    );

    handler
        .handle(FemtoLogRecord::new("core", "INFO", "one"))
        .expect("record one queued");
    handler
        .handle(FemtoLogRecord::new("core", "INFO", "two"))
        .expect("record two queued");

    assert!(handler.flush());
    handler.close();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(buffer.contents(), "core [INFO] one\ncore [INFO] two\n");
}

#[test]
fn build_from_worker_wires_handler_components() {
    let buffer = SharedBuf::default();
    let writer = buffer.clone();
    let handler_cfg = HandlerConfig {
        capacity: 1,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Block,
    };
    let policy = handler_cfg.overflow_policy;
    let mut handler = FemtoFileHandler::build_from_worker(
        writer,
        DefaultFormatter,
        handler_cfg,
        BuilderOptions::<SharedBuf>::default(),
    );

    assert!(handler.tx.is_some());
    assert!(handler.handle.is_some());
    assert_eq!(handler.overflow_policy, policy);

    let tx = handler.tx.take().expect("tx missing");
    let done_rx = handler.done_rx.clone();
    let handle = handler.handle.take().expect("handle missing");

    tx.send(FileCommand::Record(Box::new(FemtoLogRecord::new(
        "core", "INFO", "test",
    ))))
    .expect("send");
    drop(tx);

    assert!(done_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .is_ok());
    handle.join().expect("worker thread");

    assert_eq!(buffer.contents(), "core [INFO] test\n");
}

#[test]
fn femto_file_handler_invalid_file_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("missing").join("out.log");
    assert!(FemtoFileHandler::new(&path).is_err());
}

mod lifecycle_tests;
mod overflow_tests;
pub mod test_support;
mod worker_tests;
