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

    // Expand a `capacity_method` clause into a full `method` definition.
    //
    // The capacity clause is a shorthand for injecting a `with_capacity`
    // builder setter. This helper centralizes the transformation so that
    // `@process_methods` can dispatch without embedding capacity-specific
    // knowledge (doc string, method names, text signature, argument types).
    //
    // Input:  `capacity_method { self_ident = <ident>, setter = |a, b| { … } }`
    // Output: equivalent `method { … }` block forwarded to `@process_methods`
    (@expand_capacity_method
        $builder:ident,
        [$($rust_methods:tt)*],
        [$($py_methods:tt)*],
        self_ident = $self_ident:ident,
        setter = |$setter_self:ident, $setter_arg:ident| { $($setter_body:tt)* },
        remaining = [ $($rest:tt)* ],
        $( extra_py_methods = ($($extra_py_methods:tt)*) )?
    ) => {
        builder_methods!(
            @process_methods
            $builder,
            [$($rust_methods)*],
            [$($py_methods)*],
            [
                method {
                    doc: concat!(
                        "Set the bounded channel capacity.\n\n",
                        "# Validation\n\n",
                        "The capacity must be greater than zero; ",
                        "invalid values cause `build` to error.",
                    ),
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
            capacity_method {
                self_ident = $self_ident:ident,
                setter = |$setter_self:ident, $setter_arg:ident| { $($setter_body:tt)* }
            }
            $($rest:tt)*
        ],
        $( ($($extra_py_methods:tt)*) )?
    ) => {
        builder_methods!(
            @expand_capacity_method
            $builder,
            [$($rust_methods)*],
            [$($py_methods)*],
            self_ident = $self_ident,
            setter = |$setter_self, $setter_arg| { $($setter_body)* },
            remaining = [ $($rest)* ],
            $( extra_py_methods = ($($extra_py_methods)*) )?
        );
    };

    // Forward `method { ... }` clauses to the sibling clause macros, which
    // discriminate on the optional fields and emit the Rust method and the
    // Python wrapper before recursing back into `@process_methods`.
    (@process_methods
        $builder:ident,
        [$($rust_methods:tt)*],
        [$($py_methods:tt)*],
        [
            method { $($fields:tt)* }
            $(,)?
            $($rest:tt)*
        ],
        $( ($($extra_py_methods:tt)*) )?
    ) => {
        $crate::handlers::builder_macros::builder_method_clause!(
            $builder,
            [$($rust_methods)*],
            [$($py_methods)*],
            method { $($fields)* },
            remaining = [ $($rest)* ],
            extra = [ $( ($($extra_py_methods)*) )? ]
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

#[path = "builder_macros_clause_py.rs"]
mod clause_py;
#[path = "builder_macros_clause_rust.rs"]
mod clause_rust;

pub(crate) use clause_py::builder_method_clause;
pub(crate) use clause_rust::builder_method_clause_rust_args;

#[cfg(test)]
#[path = "builder_macros_tests.rs"]
mod tests;
