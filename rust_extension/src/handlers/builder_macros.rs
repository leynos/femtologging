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
/// - a Python-only `apply_*` helper that mutates the builder in place; and
/// - `#[pymethods]` exposing the helpers to Python using the provided method
///   names.
///
/// # Examples
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
///                 apply_name: apply_value,
///                 py_fn: py_with_value,
///                 py_name: "with_value",
///                 rust_args: (value: usize),
///                 py_args: (value: usize),
///                 self_ident: builder,
///                 body: {
///                     builder.value = value;
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
                        apply_name: $apply_name:ident,
                        py_fn: $py_fn:ident,
                        py_name: $py_name:literal,
                        rust_args: ( $( $rarg:ident : $rty:ty ),* $(,)? ),
                        py_args: ( $( $parg:ident : $pty:ty ),* $(,)? ),
                        self_ident: $self_ident:ident,
                        body: $body:block
                    }
                )*
            }
            $(extra_py_methods { $($extra_py_methods:tt)* })?
        }
    ) => {
        impl $builder {
            $(
                #[doc = $doc]
                pub fn $rust_name(mut self, $( $rarg : $rty ),* ) -> Self {
                    let $self_ident = &mut self;
                    $body
                    self
                }
            )*
        }

        #[cfg(feature = "python")]
        impl $builder {
            $(
                fn $apply_name(&mut self, $( $parg : $pty ),* ) {
                    let $self_ident = self;
                    $body
                }
            )*
        }

        #[cfg(feature = "python")]
        #[pyo3::pymethods]
        impl $builder {
            $(
                #[pyo3(name = $py_name)]
                fn $py_fn<'py>(mut slf: pyo3::PyRefMut<'py, Self>, $( $parg : $pty ),* ) -> pyo3::PyRefMut<'py, Self> {
                    slf.$apply_name($( $parg ),*);
                    slf
                }
            )*
            $( $($extra_py_methods)* )?
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
                    apply_name: apply_value,
                    py_fn: py_with_value,
                    py_name: "with_value",
                    rust_args: (value: usize),
                    py_args: (value: usize),
                    self_ident: builder,
                    body: {
                        builder.value = value;
                    }
                }
                method {
                    doc: "Set an optional label.",
                    rust_name: with_label,
                    apply_name: apply_label,
                    py_fn: py_with_label,
                    py_name: "with_label",
                    rust_args: (label: impl Into<String>),
                    py_args: (label: String),
                    self_ident: builder,
                    body: {
                        builder.label = Some(label.into());
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
        let builder = DummyBuilder::new().with_value(7).with_label("alpha");
        assert_eq!(builder.value, 7);
        assert_eq!(builder.label(), Some("alpha"));
    }

    #[cfg(feature = "python")]
    #[test]
    fn apply_helpers_mutate_state() {
        let mut builder = DummyBuilder::new();
        builder.apply_value(5);
        builder.apply_label("beta".to_string());
        assert_eq!(builder.value, 5);
        assert_eq!(builder.label(), Some("beta"));
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
            let guard = obj.borrow(py);
            assert_eq!(guard.value, 11);
            assert_eq!(guard.label(), Some("gamma"));
        });
    }
}
