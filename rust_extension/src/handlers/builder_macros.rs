//! Macros for generating shared builder methods.
//!
//! The builder structs expose identical fluent APIs to Rust and Python callers.
//! These macros centralise the shared method definitions so the two bindings
//! remain in sync and avoid repetitive boilerplate.

/// Validate that a value is greater than zero, returning an error otherwise.
///
/// This macro is used across handler builders for validating positive integer
/// configuration values like capacity, timeout, and size fields.
macro_rules! ensure_positive {
    ($value:expr, $field:expr) => {{
        if $value == 0 {
            Err($crate::handlers::HandlerBuildError::InvalidConfig(format!(
                "{} must be greater than zero",
                $field
            )))
        } else {
            Ok($value)
        }
    }};
}

pub(crate) use ensure_positive;

/// Set an item in a Python dict if the option contains a value.
///
/// This macro reduces boilerplate when populating Python dict representations
/// of builder configurations.
#[cfg(feature = "python")]
macro_rules! dict_set {
    ($dict:expr, $key:expr, $opt:expr) => {
        if let Some(value) = $opt {
            $dict.set_item($key, value)?;
        }
    };
}

#[cfg(feature = "python")]
pub(crate) use dict_set;

/// Generate fluent builder methods for Rust and matching Python wrappers.
///
/// The macro accepts a builder type and a list of methods. Provide an optional
/// `capacity` clause to inject a shared `with_capacity` setter before the
/// remaining method definitions. Each method is described once and the macro
/// expands to:
/// - a consuming Rust method returning `Self`;
/// - `#[pymethods]` wrappers calling the same body on a `PyRefMut` with
///   generated `#[pyo3(signature = ...)]` metadata and derived
///   `#[pyo3(text_signature = ...)]` strings;
/// - optional additional Python methods appended verbatim.
///
/// The Python signature defaults to the Rust signature; specify `py_args` only
/// when the Python API needs different argument types. Provide
/// `py_text_signature` when the generated text signature must be overridden.
/// Use `py_prelude` when a Python wrapper needs to coerce or validate its
/// arguments before running the shared method body. The builder binding defaults
/// to `builder`; set `self_ident` when a different name is clearer in the method
/// body.
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
///                 py_text_signature: "(self, label)",
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
            $(
                capacity {
                    $($capacity_tokens:tt)*
                };
            )?
            methods { $($method_tokens:tt)* }
            $(extra_py_methods { $($extra_py_methods:tt)* })?
        }
    ) => {
        builder_methods!(
            @process_methods
            $builder,
            [],
            [],
            [
                $(
                    capacity_method {
                        $($capacity_tokens)*
                    }
                )?
                $($method_tokens)*
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
        #[pymethods]
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
            capacity_method {
                self_ident = $self_ident:ident,
                setter = |$setter_self:ident, $setter_arg:ident| { $($setter_body:tt)* }
            }
            $($rest:tt)*
        ],
        $( ($($extra_py_methods:tt)*) )?
    ) => {
        builder_methods!(
            @process_methods
            $builder,
            [$($rust_methods)*],
            [$($py_methods)*],
            [
                method {
                    doc: "Set the bounded channel capacity.\n\n# Validation\n\nThe capacity must be greater than zero; invalid values cause `build` to error.",
                    rust_name: with_capacity,
                    py_fn: py_with_capacity,
                    py_name: "with_capacity",
                    py_text_signature: "(self, capacity)",
                    rust_args: (capacity: usize),
                    self_ident: $self_ident,
                    body: {
                        let $setter_self = $self_ident;
                        let $setter_arg = capacity;
                        { $($setter_body)* }
                    }
                }
                $($rest)*
            ],
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
                py_text_signature: $py_text_signature:literal,
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
                #[pyo3(text_signature = $py_text_signature)]
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
                #[pyo3(text_signature = concat!("(", "self" $(, ", ", stringify!($parg))* , ")"))]
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
                py_text_signature: $py_text_signature:literal,
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
                #[pyo3(text_signature = $py_text_signature)]
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
                #[pyo3(text_signature = concat!("(", "self" $(, ", ", stringify!($parg))* , ")"))]
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
                py_text_signature: $py_text_signature:literal,
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
                #[pyo3(text_signature = $py_text_signature)]
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
                py_text_signature: $py_text_signature:literal,
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
                #[pyo3(text_signature = $py_text_signature)]
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
                #[pyo3(text_signature = concat!("(", "self" $(, ", ", stringify!($rarg))* , ")"))]
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
    use std::num::NonZeroUsize;

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
                    py_text_signature: "(self, value)",
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
                    py_text_signature: "(self, label)",
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
                    py_text_signature: "(self)",
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
        use pyo3::types::PyAnyMethods;

        Python::with_gil(|py| {
            let obj = pyo3::Py::new(py, DummyBuilder::default())
                .expect("Py::new must create DummyBuilder");
            let any = obj.bind(py).as_any();
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

    #[cfg(feature = "python")]
    #[test]
    fn python_text_signatures_match_arguments() {
        Python::with_gil(|py| {
            let builder_type = py.get_type::<DummyBuilder>();
            let value_sig: String = builder_type
                .getattr("with_value")
                .expect("type must expose with_value")
                .getattr("__text_signature__")
                .expect("with_value must have text signature")
                .extract()
                .expect("text signature must be a string");
            assert_eq!(value_sig, "(self, value)");

            let label_sig: String = builder_type
                .getattr("with_label")
                .expect("type must expose with_label")
                .getattr("__text_signature__")
                .expect("with_label must have text signature")
                .extract()
                .expect("text signature must be a string");
            assert_eq!(label_sig, "(self, label)");

            let reset_sig: String = builder_type
                .getattr("reset")
                .expect("type must expose reset")
                .getattr("__text_signature__")
                .expect("reset must have text signature")
                .extract()
                .expect("text signature must be a string");
            assert_eq!(reset_sig, "(self)");
        });
    }

    #[cfg_attr(feature = "python", pyo3::pyclass)]
    #[derive(Clone, Debug, Default)]
    struct CapacityDummy {
        capacity: Option<usize>,
        capacity_attempted: bool,
    }

    impl CapacityDummy {
        fn new() -> Self {
            Self::default()
        }
    }

    builder_methods! {
        impl CapacityDummy {
            capacity {
                self_ident = builder,
                setter = |builder, capacity| {
                    builder.capacity_attempted = true;
                    builder.capacity = NonZeroUsize::new(capacity).map(NonZeroUsize::get);
                }
            };
            methods { }
        }
    }

    #[test]
    fn capacity_clause_generates_rust_method() {
        let builder = CapacityDummy::new().with_capacity(5);
        assert_eq!(builder.capacity, Some(5));
        assert!(
            builder.capacity_attempted,
            "with_capacity must mark that configuration occurred"
        );

        let zero_builder = CapacityDummy::new().with_capacity(0);
        assert_eq!(zero_builder.capacity, None);
        assert!(
            zero_builder.capacity_attempted,
            "with_capacity must still mark attempted configuration for zero capacity"
        );
    }

    #[cfg(feature = "python")]
    #[test]
    fn capacity_clause_generates_python_method() {
        use pyo3::types::PyAnyMethods;

        Python::with_gil(|py| {
            let obj = pyo3::Py::new(py, CapacityDummy::default())
                .expect("Py::new must create CapacityDummy");
            let any = obj.bind(py).as_any();
            any.call_method1("with_capacity", (7,))
                .expect("with_capacity must succeed for positive capacity");
            let guard = obj.borrow(py);
            assert_eq!(guard.capacity, Some(7));
            assert!(guard.capacity_attempted);
        });
    }
}
