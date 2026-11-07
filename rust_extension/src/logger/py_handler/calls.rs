//! Invocation of a wrapped Python handler's methods.
//!
//! [`PyHandler`] supports two record interfaces — the structured
//! `handle_record` and the legacy three-argument `handle` — each with a plain
//! form and a form that runs inside the producer's captured context. The
//! plumbing they share lives here: look up the method, call it, and map any
//! Python failure to a [`HandlerError`]. Flushing is also a Python method call,
//! so it is handled the same way.

use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;
use pyo3::{Py, PyAny};

use crate::formatter::python::record_to_dict;
use crate::handler::HandlerError;
use crate::log_record::FemtoLogRecord;

use super::method_call::{ContextMethodCall, DirectMethodSpec};
use super::{PyHandler, map_py_err};

impl PyHandler {
    /// Call the structured `handle_record` method with the full record dict.
    pub(super) fn call_handle_record(
        &self,
        py: Python<'_>,
        record: &FemtoLogRecord,
        context: Option<&Py<PyAny>>,
    ) -> Result<(), HandlerError> {
        let record_dict =
            record_to_dict(py, record).map_err(|err| map_py_err(py, err, "record_to_dict"))?;
        match context {
            Some(context) => self.call_handle_record_in_context(py, context, record_dict),
            None => self.call_direct_method(
                py,
                DirectMethodSpec {
                    name: "handle_record",
                    lookup_operation: "get handle_record",
                    call_operation: "handle_record",
                },
                (record_dict,),
            ),
        }
    }

    /// Runs `handle_record` inside a copied Python context and maps Python failures.
    fn call_handle_record_in_context(
        &self,
        py: Python<'_>,
        context: &Py<PyAny>,
        record_dict: Py<PyAny>,
    ) -> Result<(), HandlerError> {
        let method_caller = py
            .import("operator")
            .and_then(|operator| operator.getattr("methodcaller"))
            .and_then(|factory| factory.call1(("handle_record", record_dict)))
            .map_err(|err| map_py_err(py, err, "create handle_record caller"))?;
        self.call_methodcaller_in_context(
            py,
            context,
            ContextMethodCall::new(method_caller, "handle_record"),
        )
    }

    /// Call the legacy 3-argument `handle` method.
    pub(super) fn call_legacy_handle(
        &self,
        py: Python<'_>,
        record: &FemtoLogRecord,
        context: Option<&Py<PyAny>>,
    ) -> Result<(), HandlerError> {
        match context {
            Some(context) => self.call_legacy_handle_in_context(py, context, record),
            None => self.call_direct_method(
                py,
                DirectMethodSpec {
                    name: "handle",
                    lookup_operation: "get handle",
                    call_operation: "handle",
                },
                (record.logger(), record.level_str(), record.message()),
            ),
        }
    }

    /// Runs the legacy three-argument `handle` inside a copied Python context.
    fn call_legacy_handle_in_context(
        &self,
        py: Python<'_>,
        context: &Py<PyAny>,
        record: &FemtoLogRecord,
    ) -> Result<(), HandlerError> {
        let method_caller = py
            .import("operator")
            .and_then(|operator| operator.getattr("methodcaller"))
            .and_then(|factory| {
                factory.call1((
                    "handle",
                    record.logger(),
                    record.level_str(),
                    record.message(),
                ))
            })
            .map_err(|err| map_py_err(py, err, "create handle caller"))?;
        self.call_methodcaller_in_context(
            py,
            context,
            ContextMethodCall::new(method_caller, "handle"),
        )
    }

    /// Looks up and invokes a named Python method, preserving lookup and call errors.
    fn call_direct_method<'py>(
        &self,
        py: Python<'py>,
        method: DirectMethodSpec,
        args: impl pyo3::call::PyCallArgs<'py>,
    ) -> Result<(), HandlerError> {
        let handler_method = self
            .obj
            .bind(py)
            .getattr(method.name)
            .map_err(|err| map_py_err(py, err, method.lookup_operation))?;
        handler_method
            .call1(args)
            .map(|_| ())
            .map_err(|err| map_py_err(py, err, method.call_operation))
    }

    /// Forward a flush to a user-defined Python handler.
    ///
    /// A handler with no `flush` attribute has nothing to flush, matching the
    /// trait's no-op default. A handler that has the attribute but is not
    /// callable is treated the same way rather than as a failure: it never
    /// opted in to flushing.
    ///
    /// Python reports a failed flush by raising, not by returning a value:
    /// `logging.Handler.flush` and `io.IOBase.flush` both return `None` on
    /// success. A `None` return therefore means the handler flushed without
    /// complaint, and only an exception reports failure. Any other return
    /// value is still honoured, so a handler that explicitly returns `False`
    /// can report failure without raising.
    pub(super) fn call_python_flush(&self, py: Python<'_>) -> bool {
        let flush = match self.python_flush_method(py) {
            Ok(Some(flush)) => flush,
            Ok(None) => return true,
            Err(err) => {
                map_py_err(py, err, "get flush");
                return false;
            }
        };
        match flush.call0() {
            // Python's flush protocol reports failure by raising, not by
            // returning a value: `logging.Handler.flush` and `io.IOBase.flush`
            // both return None. None therefore means the handler flushed
            // without complaint. Any other value is honoured so a handler that
            // explicitly returns False can still report failure.
            Ok(result) if result.is_none() => true,
            Ok(result) => result.is_truthy().unwrap_or(false),
            Err(err) => {
                map_py_err(py, err, "flush");
                false
            }
        }
    }

    /// Return the handler's callable `flush`, or `None` when it opted out.
    ///
    /// A missing attribute is an opt-out, not an error: a handler that never
    /// defined `flush` has nothing to flush. So is an attribute that is not
    /// callable. Any other lookup failure is a real error the caller reports.
    fn python_flush_method<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        match self.obj.bind(py).getattr("flush") {
            Ok(flush) if flush.is_callable() => Ok(Some(flush)),
            Ok(_) => Ok(None),
            Err(err) if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Copies the captured context and invokes a prepared methodcaller on the handler.
    fn call_methodcaller_in_context<'py>(
        &self,
        py: Python<'py>,
        context: &Py<PyAny>,
        call: ContextMethodCall<'py>,
    ) -> Result<(), HandlerError> {
        let invocation_context = context
            .bind(py)
            .call_method0("copy")
            .map_err(|err| map_py_err(py, err, "copy context"))?;
        invocation_context
            .call_method1("run", (call.method_caller, self.obj.bind(py)))
            .map(|_| ())
            .map_err(|err| map_py_err(py, err, call.operation))
    }
}
