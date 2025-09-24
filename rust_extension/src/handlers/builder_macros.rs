//! Macros for generating shared builder methods.
//!
//! The builder structs expose identical fluent APIs to Rust and Python callers.
//! These macros centralise the shared method definitions so the two bindings
//! remain in sync and avoid repetitive boilerplate.

/// Generate fluent builder methods for Rust and matching Python wrappers.
///
/// The macro accepts a builder type and a list of methods. Each method is
/// described once and the macro expands to:
/// - a consuming Rust method returning `Self`;
/// - `#[pymethods]` wrappers calling the same body on a `PyRefMut` with
///   generated `#[pyo3(signature = ...)]` metadata;
/// - optional additional Python methods appended verbatim.
///
/// The Python signature defaults to the Rust signature; specify `py_args` only
/// when the Python API needs different argument types.
/// Use `py_prelude` when a Python wrapper needs to coerce or validate its
/// arguments before running the shared method body.
/// The builder binding defaults to `builder`; set `self_ident` when a
/// different name is clearer in the method body.
///
/// ```ignore
/// use pyo3::prelude::*;
///
/// #[cfg_attr(feature = "python", pyclass)]
/// #[derive(Default)]
/// struct ExampleBuilder {
///     value: usize,
/// }
///
/// builder_methods! {
///     impl ExampleBuilder {
///         methods {
///             method {
///                 doc: "Set the numeric value.",
///                 rust_name: with_value,
///                 py_fn: py_with_value,
///                 py_name: "with_value",
///                 rust_args: (value: usize),
///                 self_ident: builder,
///                 body: {
///                     builder.value = value;
///                 }
///             }
///             method {
///                 doc: "Attach a label string.",
///                 rust_name: with_label,
///                 py_fn: py_with_label,
///                 py_name: "with_label",
///                 rust_args: (label: impl Into<String>),
///                 py_args: (label: String),
///                 self_ident: builder,
///                 body: {
///                     builder.label = Some(label.into());
///                 }
///             }
///         }
///     }
/// }
///
/// let builder = ExampleBuilder::default().with_value(3);
/// assert_eq!(builder.value, 3);
/// ```
macro_rules! builder_methods {
    (
        impl $builder:ident {
            methods {
                $(
                    method {
                        doc: $doc:expr,
                        rust_name: $rust_name:ident,
                        py_fn: $py_fn:ident,
                        py_name: $py_name:literal,
                        rust_args: ( $( $rarg:ident : $rty:ty ),* $(,)? ),
                        $(py_args: ( $( $parg:ident : $pty:ty ),* $(,)? ),)?
                        $(py_prelude: { $($py_prelude:tt)* },)?
                        $(self_ident: $self_ident:ident,)?
                        body: $body:block
                    }
                )*
            }
            $(extra_py_methods { $($extra_py_methods:tt)* })?
        }
    ) => {
        builder_methods!(
            @process_methods
            $builder,
            [],
            [],
            [
                $(method {
                    doc: $doc,
                    rust_name: $rust_name,
                    py_fn: $py_fn,
                    py_name: $py_name,
                    rust_args: ( $( $rarg : $rty ),* ),
                    $(py_args: ( $( $parg : $pty ),* ),)?
                    $(py_prelude: { $($py_prelude)* },)?
                    $(self_ident: $self_ident,)?
                    body: $body
                })*
            ],
            $( ($($extra_py_methods)*) )?
        );
    };

    (@process_methods
        $builder:ident,
        [$($rust_methods:tt)*],
        [$($py_methods:tt)*],
        [],
        $( ($($extra_py_methods:tt)*) )?
    ) => {
        impl $builder {
            $($rust_methods)*
        }

        #[cfg(feature = "python")]
        #[pyo3::pymethods]
        impl $builder {
            $($py_methods)*
            $( $($extra_py_methods)* )?
        }
    };

    (@process_methods
        $builder:ident,
        [$($rust_methods:tt)*],
        [$($py_methods:tt)*],
        [
            method {
                doc: $doc:expr,
                rust_name: $rust_name:ident,
                py_fn: $py_fn:ident,
                py_name: $py_name:literal,
                rust_args: ( $( $rarg:ident : $rty:ty ),* ),
                py_args: ( $( $parg:ident : $pty:ty ),* ),
                $(py_prelude: { $($py_prelude:tt)* },)?
                body: $body:block
            }
            $(,)?
            $($rest:tt)*
        ],
        $( ($($extra_py_methods:tt)*) )?
    ) => {
        builder_methods!(
            @process_methods
            $builder,
            [
                $($rust_methods)*
                builder_methods!(@rust_method_tokens $doc, $rust_name, ( $( $rarg : $rty ),* ), builder, $body);
            ],
            [
                $($py_methods)*
                #[pyo3(name = $py_name)]
                #[pyo3(signature = ( $( $parg ),* ))]
                fn $py_fn<'py>(
                    mut slf: pyo3::PyRefMut<'py, Self>
                    $(, $parg : $pty )*
                ) -> pyo3::PyResult<pyo3::PyRefMut<'py, Self>> {
                    $( $($py_prelude)* )?
                    {
                        let builder = &mut *slf;
                        $body
                    }
                    Ok(slf)
                }
            ],
            [ $($rest)* ],
            $( ($($extra_py_methods)*) )?
        );
    };

    (@process_methods
        $builder:ident,
        [$($rust_methods:tt)*],
        [$($py_methods:tt)*],
        [
            method {
                doc: $doc:expr,
                rust_name: $rust_name:ident,
                py_fn: $py_fn:ident,
                py_name: $py_name:literal,
                rust_args: ( $( $rarg:ident : $rty:ty ),* ),
                py_args: ( $( $parg:ident : $pty:ty ),* ),
                $(py_prelude: { $($py_prelude:tt)* },)?
                self_ident: $self_ident:ident,
                body: $body:block
            }
            $(,)?
            $($rest:tt)*
        ],
        $( ($($extra_py_methods:tt)*) )?
    ) => {
        builder_methods!(
            @process_methods
            $builder,
            [
                $($rust_methods)*
                builder_methods!(@rust_method_tokens $doc, $rust_name, ( $( $rarg : $rty ),* ), $self_ident, $body);
            ],
            [
                $($py_methods)*
                #[pyo3(name = $py_name)]
                #[pyo3(signature = ( $( $parg ),* ))]
                fn $py_fn<'py>(
                    mut slf: pyo3::PyRefMut<'py, Self>
                    $(, $parg : $pty )*
                ) -> pyo3::PyResult<pyo3::PyRefMut<'py, Self>> {
                    $( $($py_prelude)* )?
                    {
                        let $self_ident = &mut *slf;
                        $body
                    }
                    Ok(slf)
                }
            ],
            [ $($rest)* ],
            $( ($($extra_py_methods)*) )?
        );
    };

    (@process_methods
        $builder:ident,
        [$($rust_methods:tt)*],
        [$($py_methods:tt)*],
        [
            method {
                doc: $doc:expr,
                rust_name: $rust_name:ident,
                py_fn: $py_fn:ident,
                py_name: $py_name:literal,
                rust_args: ( $( $rarg:ident : $rty:ty ),* ),
                $(py_prelude: { $($py_prelude:tt)* },)?
                body: $body:block
            }
            $(,)?
            $($rest:tt)*
        ],
        $( ($($extra_py_methods:tt)*) )?
    ) => {
        builder_methods!(
            @process_methods
            $builder,
            [
                $($rust_methods)*
                builder_methods!(@rust_method_tokens $doc, $rust_name, ( $( $rarg : $rty ),* ), builder, $body);
            ],
            [
                $($py_methods)*
                #[pyo3(name = $py_name)]
                #[pyo3(signature = ( $( $rarg ),* ))]
                fn $py_fn<'py>(
                    mut slf: pyo3::PyRefMut<'py, Self>
                    $(, $rarg : $rty )*
                ) -> pyo3::PyResult<pyo3::PyRefMut<'py, Self>> {
                    $( $($py_prelude)* )?
                    {
                        let builder = &mut *slf;
                        $body
                    }
                    Ok(slf)
                }
            ],
            [ $($rest)* ],
            $( ($($extra_py_methods)*) )?
        );
    };

    (@process_methods
        $builder:ident,
        [$($rust_methods:tt)*],
        [$($py_methods:tt)*],
        [
            method {
                doc: $doc:expr,
                rust_name: $rust_name:ident,
                py_fn: $py_fn:ident,
                py_name: $py_name:literal,
                rust_args: ( $( $rarg:ident : $rty:ty ),* ),
                $(py_prelude: { $($py_prelude:tt)* },)?
                self_ident: $self_ident:ident,
                body: $body:block
            }
            $(,)?
            $($rest:tt)*
        ],
        $( ($($extra_py_methods:tt)*) )?
    ) => {
        builder_methods!(
            @process_methods
            $builder,
            [
                $($rust_methods)*
                builder_methods!(@rust_method_tokens $doc, $rust_name, ( $( $rarg : $rty ),* ), $self_ident, $body);
            ],
            [
                $($py_methods)*
                #[pyo3(name = $py_name)]
                #[pyo3(signature = ( $( $rarg ),* ))]
                fn $py_fn<'py>(
                    mut slf: pyo3::PyRefMut<'py, Self>
                    $(, $rarg : $rty )*
                ) -> pyo3::PyResult<pyo3::PyRefMut<'py, Self>> {
                    $( $($py_prelude)* )?
                    {
                        let $self_ident = &mut *slf;
                        $body
                    }
                    Ok(slf)
                }
            ],
            [ $($rest)* ],
            $( ($($extra_py_methods)*) )?
        );
    };

    (@rust_method_tokens $doc:expr, $rust_name:ident, ( $( $rarg:ident : $rty:ty ),* ), $self_ident:ident, $body:block) => {
        #[doc = $doc]
        pub fn $rust_name(mut self $(, $rarg : $rty )* ) -> Self {
            let $self_ident = &mut self;
            $body
            self
        }
    };
}

pub(crate) use builder_methods;

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "python")]
    use pyo3::prelude::*;

    #[cfg_attr(feature = "python", pyo3::pyclass)]
    #[derive(Clone, Debug, Default)]
    struct DummyBuilder {
        value: usize,
        label: Option<String>,
    }

    impl DummyBuilder {
        fn new() -> Self {
            Self::default()
        }

        fn label(&self) -> Option<&str> {
            self.label.as_deref()
        }
    }

    builder_methods! {
        impl DummyBuilder {
            methods {
                method {
                    doc: "Set the stored value.",
                    rust_name: with_value,
                    py_fn: py_with_value,
                    py_name: "with_value",
                    rust_args: (value: usize),
                    self_ident: builder,
                    body: {
                        builder.value = value;
                    }
                }
                method {
                    doc: "Set an optional label.",
                    rust_name: with_label,
                    py_fn: py_with_label,
                    py_name: "with_label",
                    rust_args: (label: impl Into<String>),
                    py_args: (label: String),
                    self_ident: builder,
                    body: {
                        builder.label = Some(label.into());
                    }
                }
                method {
                    doc: "Reset to defaults.",
                    rust_name: reset,
                    py_fn: py_reset,
                    py_name: "reset",
                    rust_args: (),
                    self_ident: builder,
                    body: {
                        builder.value = 0;
                        builder.label = None;
                    }
                }
            }
            extra_py_methods {
                #[new]
                fn py_new() -> Self {
                    Self::default()
                }
            }
        }
    }

    #[test]
    fn rust_methods_chain() {
        let builder = DummyBuilder::new()
            .with_value(7)
            .with_label("alpha")
            .reset();
        assert_eq!(builder.value, 0);
        assert_eq!(builder.label(), None);
    }

    #[cfg(feature = "python")]
    #[test]
    fn python_methods_are_callable() {
        Python::with_gil(|py| {
            let obj = pyo3::Py::new(py, DummyBuilder::default())
                .expect("Py::new must create DummyBuilder");
            let any = obj.as_ref(py);
            any.call_method1("with_value", (11,))
                .expect("with_value must succeed");
            any.call_method1("with_label", ("gamma",))
                .expect("with_label must succeed");
            any.call_method0("reset").expect("reset must succeed");
            let guard = obj.borrow(py);
            assert_eq!(guard.value, 0);
            assert_eq!(guard.label(), None);
        });
    }
}
