//! Unit tests for the `log` crate bridge.

use log::{LevelFilter, Log};

use super::*;
use crate::handler::FemtoHandlerTrait;
use crate::test_utils::collecting_handler::CollectingHandler;
use rstest::{fixture, rstest};
use serial_test::serial;
use std::sync::{
    Arc, Once,
    atomic::{AtomicUsize, Ordering},
};

static LOGGER_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[fixture]
fn unique_logger_name() -> String {
    let base = "bridge.test";
    let suffix = LOGGER_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{base}.{suffix}")
}

#[rstest]
#[case(log::Level::Trace, FemtoLevel::Trace)]
#[case(log::Level::Debug, FemtoLevel::Debug)]
#[case(log::Level::Info, FemtoLevel::Info)]
#[case(log::Level::Warn, FemtoLevel::Warn)]
#[case(log::Level::Error, FemtoLevel::Error)]
fn level_mapping_is_direct(#[case] level: log::Level, #[case] expected: FemtoLevel) {
    assert_eq!(FemtoLevel::from(level), expected);
}

#[fixture]
fn log_max_level() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        log::set_max_level(LevelFilter::Trace);
    });
}

#[rstest]
#[serial]
fn adapter_dispatches_records_to_target_logger(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger.borrow(py).add_handler(handler.clone());

        let record = log::Record::builder()
            .args(format_args!("hello"))
            .level(log::Level::Info)
            .target(&logger_name)
            .module_path(Some("bridge::test"))
            .file(Some("lib.rs"))
            .line(Some(42))
            .build();

        adapter.log(&record);

        assert!(
            logger.borrow(py).flush_handlers(),
            "flush should drain the queue"
        );

        let records = handler
            .as_any()
            .downcast_ref::<CollectingHandler>()
            .expect("handler downcast")
            .collected();
        assert_eq!(records.len(), 1);
        let rec = &records[0];
        assert_eq!(rec.logger(), logger_name.as_str());
        assert_eq!(rec.level_str(), "INFO");
        assert_eq!(rec.message(), "hello");
        assert_eq!(rec.metadata().module_path, "bridge::test");
        assert_eq!(rec.metadata().filename, "lib.rs");
        assert_eq!(rec.metadata().line_number, 42);
    });
}

#[rstest]
#[serial]
fn adapter_normalizes_rust_module_targets(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;
    let target = logger_name.replace('.', "::");

    Python::attach(|py| {
        let logger = manager::get_logger(py, &logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger.borrow(py).add_handler(handler.clone());

        let record = log::Record::builder()
            .args(format_args!("normalized"))
            .level(log::Level::Info)
            .target(&target)
            .build();

        adapter.log(&record);
        assert!(logger.borrow(py).flush_handlers());

        let records = handler
            .as_any()
            .downcast_ref::<CollectingHandler>()
            .expect("handler downcast")
            .collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].logger(), logger_name.as_str());
    });
}

#[rstest]
#[serial]
fn log_respects_logger_threshold(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger.borrow(py).add_handler(handler.clone());
        logger.borrow(py).set_level(FemtoLevel::Warn);

        let info_record = log::Record::builder()
            .args(format_args!("info"))
            .level(log::Level::Info)
            .target(&logger_name)
            .build();
        adapter.log(&info_record);

        let warn_record = log::Record::builder()
            .args(format_args!("warn"))
            .level(log::Level::Warn)
            .target(&logger_name)
            .build();
        adapter.log(&warn_record);

        assert!(
            logger.borrow(py).flush_handlers(),
            "flush should drain the queue"
        );

        let records = handler
            .as_any()
            .downcast_ref::<CollectingHandler>()
            .expect("handler downcast")
            .collected();
        assert_eq!(records.len(), 1, "only WARN should pass threshold");
        assert_eq!(records[0].level_str(), "WARN");
    });
}

/// Every target the bridge cannot register still reaches `root`.
///
/// A high-cardinality target stream is refused by the registry once the cap is
/// reached, and a malformed one by name validation. Both refusals travel the
/// same `Err` arm, and in both the record must survive: the bridge falls back
/// to `root` rather than dropping the record or panicking. The targets here are
/// refused by validation rather than by filling the registry, because the
/// registry is process-global and a unit test that saturated it would refuse
/// its neighbours' loggers too.
///
/// The `log_max_level` fixture is required, not decorative: `log` defaults to
/// `Off`, and the adapter returns before resolving a target below it, so
/// without the fixture this case would assert nothing.
#[rstest]
#[serial]
fn refused_targets_fall_back_to_root_without_losing_records(_log_max_level: ()) {
    manager::reset_manager();

    let refused = [
        // Longer than `MAX_LOGGER_NAME_BYTES`, so validation refuses it.
        format!(
            "stress.{}",
            "a".repeat(crate::manager::MAX_LOGGER_NAME_BYTES)
        ),
        // A newline would let a target forge a line boundary in output.
        "stress.forged\nfake [ERROR] injected".to_string(),
        // Deeper than `MAX_LOGGER_NAME_SEGMENTS`.
        ["deep"; crate::manager::MAX_LOGGER_NAME_SEGMENTS + 1].join("."),
    ];

    Python::attach(|py| {
        let adapter = FemtoLogAdapter;
        let root = manager::get_logger(py, "root").expect("root is always resolvable");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        root.borrow(py).clear_handlers();
        root.borrow(py).add_handler(handler.clone());

        for target in &refused {
            // `format_args!` builds a temporary `Arguments` borrow; binding it
            // first is what lets the record outlive the builder statement.
            let args = format_args!("refused {target:?}");
            let record = log::Record::builder()
                .args(args)
                .level(log::Level::Info)
                .target(target)
                .build();
            adapter.log(&record);
        }
        assert!(root.borrow(py).flush_handlers(), "flush should drain");

        let collected = handler
            .as_any()
            .downcast_ref::<CollectingHandler>()
            .expect("handler downcast")
            .collected();
        assert_eq!(
            collected.len(),
            refused.len(),
            "each refused target's record must reach root, not be dropped"
        );
        assert!(
            collected.iter().all(|record| record.logger() == "root"),
            "a refused target's record must be attributed to the fallback logger"
        );
    });

    manager::reset_manager();
}

/// A record the bridge logs about its own work does not re-enter the bridge.
///
/// The adapter is installed as the global Rust logger, so the diagnostic it
/// emits when a target is refused would be routed straight back into the
/// adapter unless its own targets are ignored. It is exactly the wrong moment
/// to be unguarded: the diagnostic fires because the caller is emitting many
/// distinct targets, so each warning would beget another record. The tracing
/// bridge already ignores its own targets; this asserts the `log` bridge does
/// too, and asserts it with a target that would otherwise resolve.
///
/// The `log_max_level` fixture is required: `log` defaults to `Off`, and the
/// adapter returns before consulting the ignore list below it, so without the
/// fixture the assertion would hold for the wrong reason.
#[rstest]
#[serial]
fn the_bridge_ignores_records_about_itself(_log_max_level: ()) {
    manager::reset_manager();

    Python::attach(|py| {
        let adapter = FemtoLogAdapter;
        let internal = manager::get_logger(py, "femtologging.log_compat")
            .expect("the internal logger is registrable");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        internal.borrow(py).clear_handlers();
        internal.borrow(py).add_handler(handler.clone());

        for target in [
            "femtologging",
            "femtologging.log_compat",
            "femtologging::inner",
        ] {
            let record = log::Record::builder()
                .args(format_args!("internal note"))
                .level(log::Level::Warn)
                .target(target)
                .build();
            adapter.log(&record);
        }

        assert!(
            internal.borrow(py).flush_handlers(),
            "flush should drain the internal logger"
        );
        assert!(
            handler
                .as_any()
                .downcast_ref::<CollectingHandler>()
                .expect("handler downcast")
                .collected()
                .is_empty(),
            "a femtologging-owned target must not be forwarded back through the bridge"
        );
    });

    manager::reset_manager();
}
