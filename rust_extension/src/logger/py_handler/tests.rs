//! Tests for native handler recognition and flushing in the Python wrapper.

use super::{NativeHandlerKind, PyHandler};
use crate::handler::FemtoHandlerTrait;
use crate::{
    FemtoFileHandler, FemtoHTTPHandler, FemtoSocketHandler, HTTPHandlerConfig, SocketHandlerConfig,
};
use pyo3::types::{PyAny, PyAnyMethods, PyModule};
use pyo3::{Py, Python};
use rstest::rstest;
use std::ffi::CString;

/// Python handler classes covering each shape of `flush` the wrapper meets.
///
/// Every class counts its flush calls on the instance, never in module-level
/// state, so cases that run in parallel cannot observe each other's calls.
const HANDLER_SOURCE: &str = concat!(
    "class Base:\n",
    "    def __init__(self):\n",
    "        self.calls = 0\n",
    "\n",
    "    def handle(self, logger, level, message):\n",
    "        pass\n",
    "\n",
    "\n",
    "class Succeeding(Base):\n",
    "    def flush(self):\n",
    "        self.calls += 1\n",
    "        return True\n",
    "\n",
    "\n",
    "class Failing(Base):\n",
    "    def flush(self):\n",
    "        self.calls += 1\n",
    "        return False\n",
    "\n",
    "\n",
    "class Raising(Base):\n",
    "    def flush(self):\n",
    "        self.calls += 1\n",
    "        raise RuntimeError('flush failed')\n",
    "\n",
    "\n",
    "class NoneReturning(Base):\n",
    "    def flush(self):\n",
    "        self.calls += 1\n",
    "        return None\n",
    "\n",
    "\n",
    "class ZeroReturning(Base):\n",
    "    def flush(self):\n",
    "        self.calls += 1\n",
    "        return 0\n",
    "\n",
    "\n",
    "class WithoutFlush(Base):\n",
    "    pass\n",
    "\n",
    "\n",
    "class NonCallableFlush(Base):\n",
    "    flush = 42\n",
    "\n",
    "\n",
    "class BrokenFlushLookup(Base):\n",
    "    @property\n",
    "    def flush(self):\n",
    "        raise RuntimeError('flush lookup failed')\n",
);

/// Build a wrapper around an instance of the named class in [`HANDLER_SOURCE`].
///
/// The instance is returned with the wrapper so a test can read its `calls`
/// counter after flushing. Each class gets its own module name, and the counter
/// lives on the instance, so parallel cases cannot share flush state.
fn user_defined_handler<'py>(py: Python<'py>, class_name: &str) -> (PyHandler, Py<PyAny>) {
    let c_string = |value: &str| CString::new(value).expect("test source has no NUL bytes");
    let module = PyModule::from_code(
        py,
        c_string(HANDLER_SOURCE).as_c_str(),
        c_string("user_handlers.py").as_c_str(),
        c_string(class_name).as_c_str(),
    )
    .expect("handler source should compile");
    let instance = module
        .getattr(class_name)
        .expect("handler class should exist")
        .call0()
        .expect("handler class should instantiate")
        .unbind();
    let wrapper = PyHandler::new(py, instance.clone_ref(py));
    (wrapper, instance)
}

/// Read the counter of calls into a user-defined handler's `flush`.
fn flush_calls(handler: &Py<PyAny>, py: Python<'_>) -> u32 {
    handler
        .bind(py)
        .getattr("calls")
        .expect("counter should exist")
        .extract()
        .expect("counter should be an integer")
}

/// Built-in socket and HTTP handlers are recognized for native dispatch.
#[test]
fn native_handler_kind_recognizes_network_handlers() {
    Python::attach(|py| {
        let socket = Py::new(
            py,
            FemtoSocketHandler::with_config(SocketHandlerConfig::default()),
        )
        .expect("socket handler should be constructible as a Python object");
        assert!(matches!(
            NativeHandlerKind::from_object(socket.bind(py)),
            Some(NativeHandlerKind::Socket)
        ));

        let http = Py::new(
            py,
            FemtoHTTPHandler::with_config(HTTPHandlerConfig::default()),
        )
        .expect("HTTP handler should be constructible as a Python object");
        assert!(matches!(
            NativeHandlerKind::from_object(http.bind(py)),
            Some(NativeHandlerKind::Http)
        ));
    });
}

/// A built-in handler registered through Python is flushed by the wrapper.
///
/// A closed file handler reports a failed flush, which distinguishes the
/// delegated flush from the trait's default no-op success.
#[test]
fn py_handler_flushes_a_native_file_handler() {
    Python::attach(|py| {
        let dir = tempfile::tempdir().expect("temporary directory");
        let handler =
            FemtoFileHandler::new(dir.path().join("app.log")).expect("file handler should open");
        handler.close();
        let obj = Py::new(py, handler)
            .expect("file handler should be constructible as a Python object")
            .into_any();
        let wrapper = PyHandler::new(py, obj);
        assert!(
            !wrapper.flush(),
            "a closed native handler must report a failed flush"
        );
    });
}

/// A flush attempt that cannot attach the interpreter reports failure.
///
/// `Python::try_attach` returns `None` only while the interpreter is
/// finalizing, which a test cannot arrange, so the attempt is injected. The
/// wrapper must not claim success over records it never asked the handler to
/// write.
#[test]
fn py_handler_reports_failure_when_the_interpreter_cannot_be_attached() {
    Python::attach(|py| {
        let (wrapper, _handler) = user_defined_handler(py, "Succeeding");
        assert!(
            !wrapper.report_flush(|| None),
            "an unattachable interpreter must report a failed flush"
        );
    });
}

/// One shape of user-defined `flush`, and what the wrapper must report for it.
struct FlushCase {
    /// Class in [`HANDLER_SOURCE`] to instantiate.
    class_name: &'static str,
    /// Whether the wrapper's flush must report success.
    succeeds: bool,
    /// How many times the handler's own `flush` must have run.
    expected_calls: u32,
}

/// Every flush shape a user-defined handler can present to the wrapper.
///
/// A missing or non-callable `flush` counts as success because Python handlers
/// only have to implement `handle`. A failed lookup is different: the wrapper
/// could not establish that there was nothing to flush, so it reports failure.
#[rstest]
#[case::successful(FlushCase { class_name: "Succeeding", succeeds: true, expected_calls: 1 })]
#[case::returning_false(FlushCase { class_name: "Failing", succeeds: false, expected_calls: 1 })]
#[case::raising(FlushCase { class_name: "Raising", succeeds: false, expected_calls: 1 })]
#[case::returning_none(FlushCase { class_name: "NoneReturning", succeeds: true, expected_calls: 1 })]
#[case::returning_zero(FlushCase { class_name: "ZeroReturning", succeeds: true, expected_calls: 1 })]
#[case::without_flush(FlushCase { class_name: "WithoutFlush", succeeds: true, expected_calls: 0 })]
#[case::non_callable(FlushCase { class_name: "NonCallableFlush", succeeds: true, expected_calls: 0 })]
#[case::failed_lookup(FlushCase { class_name: "BrokenFlushLookup", succeeds: false, expected_calls: 0 })]
fn py_handler_reports_each_user_defined_flush_shape(#[case] case: FlushCase) {
    Python::attach(|py| {
        let (wrapper, handler) = user_defined_handler(py, case.class_name);
        assert_eq!(
            wrapper.flush(),
            case.succeeds,
            "{} reported the wrong flush result",
            case.class_name
        );
        assert_eq!(
            flush_calls(&handler, py),
            case.expected_calls,
            "{} did not receive the expected flush calls",
            case.class_name
        );
    });
}
