//! Regression tests for flushing Python handlers through the Rust `log` bridge.
//!
//! The `log` adapter in the parent `log_compat` module forwards records to a
//! `FemtoLogger`, and a Python handler attached to that logger may need the
//! manager lock in order to run its own `flush`. The scenario installs a gated
//! handler that blocks until the test releases it, then proves the adapter's
//! flush completes instead of deadlocking on the manager write lock.
//! `create_gated_handler` builds the handler, `spawn_manager_lock_probe`
//! watches for the lock, and
//! `adapter_flush_releases_manager_for_python_handlers` drives the case;
//! `unique_logger_name` keeps concurrent runs from sharing a logger.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use log::Log;
use pyo3::prelude::{Py, PyAny, PyResult, Python};
use pyo3::types::{PyAnyMethods, PyModule};
use rstest::rstest;
use serial_test::serial;

use crate::level::FemtoLevel;
use crate::manager;

use super::FemtoLogAdapter;
use super::test_support::unique_logger_name;

fn create_gated_handler(py: Python<'_>) -> PyResult<(Py<PyAny>, Py<PyAny>, Py<PyAny>)> {
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
    let filename =
        std::ffi::CString::new("gated_handler.py").expect("test filename contains no NUL bytes");
    let module_name =
        std::ffi::CString::new("gated_handler").expect("test module name contains no NUL bytes");
    let module = PyModule::from_code(
        py,
        source.as_c_str(),
        filename.as_c_str(),
        module_name.as_c_str(),
    )?;
    let handler = module.getattr("handler")?.unbind();
    let started = handler.getattr(py, "started")?;
    let release = handler.getattr(py, "release")?;

    Ok((handler, started, release))
}

fn wait_for_manager_write_lock(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if manager::manager_write_available_for_test() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn spawn_manager_lock_probe(
    started: Py<PyAny>,
    release: Py<PyAny>,
) -> (std::thread::JoinHandle<()>, Arc<AtomicBool>) {
    let lock_available_during_flush = Arc::new(AtomicBool::new(false));
    let helper_lock_available = Arc::clone(&lock_available_during_flush);
    let lock_probe = std::thread::spawn(move || {
        Python::attach(|py| {
            let callback_started = started
                .bind(py)
                .call_method1("wait", (5.0,))
                .and_then(|value| value.extract::<bool>())
                .unwrap_or(false);
            if callback_started {
                let manager_lock_was_available =
                    py.detach(|| wait_for_manager_write_lock(Duration::from_secs(3)));
                helper_lock_available.store(manager_lock_was_available, Ordering::Release);
            }
            drop(release.bind(py).call_method0("set"));
        });
    });

    (lock_probe, lock_available_during_flush)
}

#[rstest]
#[serial]
fn adapter_flush_releases_manager_for_python_handlers(unique_logger_name: String) -> PyResult<()> {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;
    Python::attach(|py| -> PyResult<()> {
        let (handler, started, release) = create_gated_handler(py)?;
        let logger = manager::get_logger(py, &logger_name)?;
        logger.borrow(py).py_add_handler(handler.clone_ref(py))?;
        logger
            .borrow(py)
            .log(FemtoLevel::Info, "before bridge flush");

        let (lock_probe, lock_available_during_flush) =
            spawn_manager_lock_probe(started, release.clone_ref(py));

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
        logger.borrow(py).py_remove_handler(handler.bind(py));
        let helper_joined = py.detach(|| lock_probe.join()).is_ok();

        if !helper_joined {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "manager lock probe thread should finish",
            ));
        }
        if !cleanup_flushed {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "cleanup should drain the Python handler",
            ));
        }
        if !delivered_before_bridge_return {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "log bridge flush should deliver the queued Python callback before returning",
            ));
        }
        if !manager_lock_was_available {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "manager lock should be available while the Python handler is gated",
            ));
        }
        Ok(())
    })
}
