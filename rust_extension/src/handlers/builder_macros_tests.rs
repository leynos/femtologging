//! Tests for the shared builder-method macros.

use std::num::NonZeroUsize;

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg_attr(feature = "python", pyo3::pyclass(from_py_object))]
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

    Python::attach(|py| {
        let obj =
            pyo3::Py::new(py, DummyBuilder::default()).expect("Py::new must create DummyBuilder");
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
    Python::attach(|py| {
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

#[cfg_attr(feature = "python", pyo3::pyclass(from_py_object))]
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

    Python::attach(|py| {
        let obj =
            pyo3::Py::new(py, CapacityDummy::default()).expect("Py::new must create CapacityDummy");
        let any = obj.bind(py).as_any();
        any.call_method1("with_capacity", (7,))
            .expect("with_capacity must succeed for positive capacity");
        let guard = obj.borrow(py);
        assert_eq!(guard.capacity, Some(7));
        assert!(guard.capacity_attempted);
    });
}
