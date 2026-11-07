//! Tests for native handler recognition and flushing in the Python wrapper.

use super::{NativeHandlerKind, PyHandler};
use crate::handler::FemtoHandlerTrait;
use crate::{
    FemtoFileHandler, FemtoHTTPHandler, FemtoSocketHandler, HTTPHandlerConfig, SocketHandlerConfig,
};
use pyo3::{Py, Python};

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
