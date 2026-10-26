//! GIL and deadline behavior of the manager's logger flush bridge.

use std::ffi::CString;
use std::time::Duration;

use pyo3::types::{PyAnyMethods, PyModule};
use pyo3::{Py, PyAny, Python};
use serial_test::serial;

use crate::manager::{flush_all_handlers, get_logger, reset_manager};
use crate::sync::{JoinHandle, spawn};
use crate::level::FemtoLevel;

/// Releases Python callbacks and clears manager-owned workers on unwind.
struct ManagerFlushCleanup<'py> {
    /// Interpreter token for cleanup after a failed assertion.
    py: Python<'py>,
    /// Python event that releases the worker's test callback.
    release_event: Py<PyAny>,
    /// Thread that releases the callback while the manager barrier detaches.
    release_thread: Option<JoinHandle<()>>,
}

impl Drop for ManagerFlushCleanup<'_> {
    fn drop(&mut self) {
        let _ = self.release_event.bind(self.py).call_method0("set");
        let release_thread = self.release_thread.take();
        self.py.detach(move || {
            if let Some(release_thread) = release_thread {
                let _ = release_thread.join();
            }
            reset_manager();
        });
    }
}

/// The manager bridge releases the GIL while a queued Python callback is active.
#[test]
#[serial]
fn flush_all_handlers_releases_the_gil_for_a_queued_python_handler() -> pyo3::PyResult<()> {
    Python::attach(|py| -> pyo3::PyResult<()> {
        reset_manager();

        let threading = py.import("threading")?;
        let started = threading.call_method0("Event")?;
        let release = threading.call_method0("Event")?;
        let finished = threading.call_method0("Event")?;
        let source = CString::new(concat!(
            "class Handler:\n",
            "    def __init__(self, started, release, finished):\n",
            "        self.started = started\n",
            "        self.release = release\n",
            "        self.finished = finished\n",
            "    def handle(self, name, level, message):\n",
            "        self.started.set()\n",
            "        self.release.wait(5.0)\n",
            "        self.finished.set()\n",
        ))
        .expect("Python handler source contains no null bytes");
        let module = PyModule::from_code(
            py,
            source.as_c_str(),
            c"manager_flush_test.py",
            c"manager_flush_test",
        )?;
        let handler = module
            .getattr("Handler")?
            .call1((&started, &release, &finished))?;
        let mut cleanup = ManagerFlushCleanup {
            py,
            release_event: release.clone().unbind(),
            release_thread: None,
        };

        let logger = get_logger(py, "bridge.flush.python")?;
        logger.borrow(py).py_add_handler(handler.clone().unbind())?;
        logger.borrow(py).py_log(
            py,
            FemtoLevel::Info,
            "queued before manager flush",
            None,
            None,
            None,
        )?;

        let callback_started = started.call_method1("wait", (1.0,))?.extract::<bool>()?;
        assert!(
            callback_started,
            "the worker should enter the Python callback before the manager flush"
        );

        let release_for_thread = release.clone().unbind();
        cleanup.release_thread = Some(spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            Python::attach(|py| {
                release_for_thread
                    .bind(py)
                    .call_method0("set")
                    .expect("release event should be set");
            });
        }));

        assert!(
            flush_all_handlers(py),
            "manager flush should confirm after the queued Python callback completes"
        );
        assert!(
            finished.call_method0("is_set")?.extract::<bool>()?,
            "the queued Python callback must finish before the manager barrier confirms"
        );
        Ok(())
    })
}
