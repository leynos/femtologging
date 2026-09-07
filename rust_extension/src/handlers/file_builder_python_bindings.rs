//! Generated Rust and Python methods for [`super::FileHandlerBuilder`].
//!
//! The builder macro emits both direct Rust fluent methods and `PyO3` wrappers;
//! keeping its invocation private isolates the generated Python expansion.

#[cfg(feature = "python")]
use pyo3::prelude::*;

use super::FileHandlerBuilder;
#[cfg(feature = "python")]
use super::{FemtoFileHandler, HandlerBuilderTrait};
#[cfg(feature = "python")]
use super::{PyOverflowPolicy, py_flush_after_records_to_nonzero};
use crate::handlers::builder_macros::builder_methods;
#[cfg(feature = "python")]
use crate::macros::AsPyDict;
use std::num::NonZeroU64;

builder_methods! {
    impl FileHandlerBuilder {
        capacity {
            const = true,
            self_ident = builder,
            setter = |builder_ref, capacity| {
                builder_ref.common.set_capacity(capacity);
            }
        };
        methods {
            method {
                doc: "Set the flush threshold measured in records.\n\n# Validation\n\nThe threshold must be greater than zero (`NonZeroU64`).\n\n# Platform-specific behaviour\n\nOn 32-bit platforms where `usize::MAX < u64::MAX`, values exceeding `usize::MAX` are clamped silently at build time. Python callers receive an `OverflowError` instead (validated at the API boundary).",
                rust_name: with_flush_after_records,
                py_fn: py_with_flush_after_records,
                py_name: "with_flush_after_records",
                py_text_signature: "(self, interval)",
                rust_args: (parsed_interval: NonZeroU64),
                py_args: (interval: u64),
                rust_const: true,
                py_prelude: {
                    let parsed_interval =
                        py_flush_after_records_to_nonzero(interval)?;
                },
                self_ident: builder,
                body: {
                    builder.common.set_flush_after_records(parsed_interval);
                }
            }
        }
        extra_py_methods {
            /// Create a new `FileHandlerBuilder`.
            ///
            /// Mirrors Python's `logging.FileHandler` constructor by accepting the
            /// filesystem path directly so Python callers can pass the same
            /// `filename` argument.
            #[new]
            fn py_new(path: &str) -> Self {
                Self::new(path)
            }

            #[pyo3(name = "with_overflow_policy")]
            fn py_with_overflow_policy(
                mut slf: PyRefMut<'_, Self>,
                policy: PyOverflowPolicy,
            ) -> PyRefMut<'_, Self> {
                slf.common.set_overflow_policy(policy.inner);
                slf
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
            fn build(&self) -> PyResult<FemtoFileHandler> {
                <Self as HandlerBuilderTrait>::build_inner(self).map_err(PyErr::from)
            }
        }
    }
}
