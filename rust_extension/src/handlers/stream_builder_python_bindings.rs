//! Generated Rust and Python methods for [`super::StreamHandlerBuilder`].
//!
//! The builder macro emits both direct Rust fluent methods and `PyO3` wrappers;
//! keeping its invocation private isolates the generated Python expansion.

use super::*;
use crate::handlers::builder_macros::builder_methods;

builder_methods! {
    impl StreamHandlerBuilder {
        capacity {
            const = true,
            self_ident = builder,
            setter = |builder_ref, capacity| {
                builder_ref.common.set_capacity(capacity);
            }
        };
        methods {
            method {
                doc: "Set the flush threshold in milliseconds.\n\n# Validation\n\nAccepts a `NonZeroU64` so both Rust and Python callers must provide a value greater than zero.",
                rust_name: with_flush_after_ms,
                py_fn: py_with_flush_after_ms,
                py_name: "with_flush_after_ms",
                py_text_signature: "(self, flush_ms)",
                rust_args: (parsed_flush_ms: NonZeroU64),
                py_args: (flush_ms: u64),
                rust_const: true,
                py_prelude: {
                    let parsed_flush_ms = NonZeroU64::new(flush_ms).ok_or_else(|| {
                        pyo3::exceptions::PyValueError::new_err(
                            "flush_after_ms must be greater than zero",
                        )
                    })?;
                },
                self_ident: builder,
                body: {
                    builder.common.flush_after_ms = Some(parsed_flush_ms);
                }
            }
        }
        extra_py_methods {
            /// Create a new `StreamHandlerBuilder` defaulting to `stderr`.
            ///
            /// Mirrors Python's `logging.StreamHandler` default stream.
            #[new]
            fn py_new() -> Self {
                Self::stderr()
            }

            #[staticmethod]
            #[pyo3(name = "stdout")]
            fn py_stdout() -> Self {
                Self::stdout()
            }

            #[staticmethod]
            #[pyo3(name = "stderr")]
            fn py_stderr() -> Self {
                Self::stderr()
            }

            #[pyo3(name = "with_formatter")]
            #[pyo3(signature = (formatter))]
            #[pyo3(text_signature = "(self, formatter)")]
            fn py_with_formatter<'py>(
                mut slf: PyRefMut<'py, Self>,
                formatter: &Bound<'py, PyAny>,
            ) -> PyResult<PyRefMut<'py, Self>> {
                slf.common.set_formatter_from_py(formatter)?;
                Ok(slf)
            }

            /// Return a dictionary describing the builder configuration.
            fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
                self.as_pydict(py)
            }

            /// Build the handler, raising ``HandlerConfigError`` or ``HandlerIOError`` on
            /// failure.
            fn build(&self) -> PyResult<FemtoStreamHandler> {
                <Self as HandlerBuilderTrait>::build_inner(self).map_err(PyErr::from)
            }
        }
    }
}
