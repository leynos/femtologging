//! Unit tests for the `log` crate bridge.

use log::{LevelFilter, Log};

use super::*;
use crate::handler::FemtoHandlerTrait;
use crate::test_utils::collecting_handler::CollectingHandler;
use rstest::{fixture, rstest};
use std::sync::{
    Arc, Once,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

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
fn adapter_dispatches_records_to_target_logger(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger
            .borrow(py)
            .add_handler(handler.clone())
            .expect("native test handler should register");

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
fn adapter_normalizes_rust_module_targets(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;
    let target = logger_name.replace('.', "::");

    Python::attach(|py| {
        let logger = manager::get_logger(py, &logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger
            .borrow(py)
            .add_handler(handler.clone())
            .expect("native test handler should register");

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
fn adapter_flush_releases_manager_for_python_handlers(
    unique_logger_name: String,
) -> pyo3::PyResult<()> {
    let adapter = FemtoLogAdapter;
    Python::attach(|py| -> pyo3::PyResult<()> {
        let source = std::ffi::CString::new(concat!(
            "from threading import Event\n",
            "class GatedHandler:\n",
            "    def __init__(self):\n",
            "        self.started = Event()\n",
            "        self.release = Event()\n",
            "        self.records = []\n",
            "    def handle_record(self, record):\n",
            "        self.started.set()\n",
            "        if not self.release.wait(timeout=5.0):\n",
            "            raise TimeoutError('handler gate timed out')\n",
            "        self.records.append(record)\n",
            "    def handle(self, logger, level, message):\n",
            "        self.records.append((logger, level, message))\n",
            "handler = GatedHandler()\n",
        ))
        .expect("test handler source contains no NUL bytes");
        let filename = std::ffi::CString::new("gated_handler.py")
            .expect("test filename contains no NUL bytes");
        let module_name = std::ffi::CString::new("gated_handler")
            .expect("test module name contains no NUL bytes");
        let module = PyModule::from_code(
            py,
            source.as_c_str(),
            filename.as_c_str(),
            module_name.as_c_str(),
        )?;
        let handler = module.getattr("handler")?.unbind();
        let started = handler.getattr(py, "started")?;
        let release = handler.getattr(py, "release")?;
        let lock_available_during_flush = Arc::new(AtomicBool::new(false));
        let helper_lock_available = Arc::clone(&lock_available_during_flush);
        let helper_started = started.clone_ref(py);
        let helper_release = release.clone_ref(py);

        let logger = manager::get_logger(py, &unique_logger_name)?;
        logger.borrow(py).py_add_handler(handler.clone_ref(py))?;
        logger
            .borrow(py)
            .log(FemtoLevel::Info, "before bridge flush");

        let lock_probe = std::thread::spawn(move || {
            Python::attach(|py| {
                let callback_started = helper_started
                    .bind(py)
                    .call_method1("wait", (5.0,))
                    .and_then(|value| value.extract::<bool>())
                    .unwrap_or(false);
                if callback_started {
                    let manager_lock_was_available = py.detach(|| {
                        let deadline = Instant::now() + Duration::from_secs(3);
                        loop {
                            if manager::manager_write_available_for_test() {
                                break true;
                            }
                            if Instant::now() >= deadline {
                                break false;
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    });
                    helper_lock_available.store(manager_lock_was_available, Ordering::Release);
                }
                let _ = helper_release.bind(py).call_method0("set");
            });
        });

        // Keep Python attached from enqueue through the log bridge flush.
        adapter.flush();
        let delivered_before_bridge_return = handler.bind(py).getattr("records")?.len()? == 1;
        let manager_lock_was_available = lock_available_during_flush.load(Ordering::Acquire);

        release.bind(py).call_method0("set")?;
        let cleanup_flushed = {
            let borrowed_logger = logger.borrow(py);
            let logger_ref: &crate::FemtoLogger = &borrowed_logger;
            py.detach(|| logger_ref.flush_handlers())
        };
        logger.borrow(py).py_remove_handler(handler.clone_ref(py));
        let helper_joined = py.detach(|| lock_probe.join()).is_ok();

        assert!(helper_joined, "manager lock probe thread should finish");
        assert!(cleanup_flushed, "cleanup should drain the Python handler");
        assert!(
            delivered_before_bridge_return,
            "log bridge flush should deliver the queued Python callback before returning",
        );
        assert!(
            manager_lock_was_available,
            "manager lock should be available while the Python handler is gated",
        );
        Ok(())
    })
}

#[rstest]
fn log_respects_logger_threshold(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger
            .borrow(py)
            .add_handler(handler.clone())
            .expect("native test handler should register");
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
