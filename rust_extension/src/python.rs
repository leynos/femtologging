#![cfg(feature = "python")]

//! Shared helpers for Python interaction.
//!
//! Utilities to keep Python-only code deduplicated across modules.

use pyo3::prelude::*;

/// Return the fully qualified Python type name for `obj`.
///
/// Falls back to `"<unknown>"` when the module or qualified name cannot
/// be retrieved.
pub(crate) fn fq_py_type(obj: &Bound<'_, PyAny>) -> String {
    let ty = obj.get_type();
    let module = ty
        .getattr("__module__")
        .and_then(|m| m.extract::<String>())
        .unwrap_or_else(|_| "<unknown>".to_string());
    let qualname = ty
        .getattr("__qualname__")
        .and_then(|n| n.extract::<String>())
        .unwrap_or_else(|_| "<unknown>".to_string());
    if module == "builtins" {
        qualname
    } else {
        format!("{module}.{qualname}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::types::{PyList, PyModule};
    use std::ffi::CString;

    #[test]
    fn returns_builtin_name_without_module() {
        Python::with_gil(|py| {
            let list = PyList::empty(py);
            let name = fq_py_type(list.as_any());
            assert_eq!(name, "list");
        });
    }

    #[test]
    fn returns_user_defined_fq_name() {
        Python::with_gil(|py| {
            let module = PyModule::from_code(
                py,
                "class Foo: pass\n",
                CString::new("mymod.py").unwrap(),
                CString::new("mymod").unwrap(),
            )
            .unwrap();
            let obj = module.getattr("Foo").unwrap().call0().unwrap();
            let name = fq_py_type(&obj);
            assert_eq!(name, "mymod.Foo");
        });
    }

    #[test]
    fn falls_back_when_attrs_missing() {
        Python::with_gil(|py| {
            let module = PyModule::from_code(
                py,
                "class Bar: pass\n",
                CString::new("mymod.py").unwrap(),
                CString::new("mymod").unwrap(),
            )
            .unwrap();
            let class = module.getattr("Bar").unwrap();
            class.delattr("__module__").unwrap();
            class.delattr("__qualname__").unwrap();
            let obj = class.call0().unwrap();
            let name = fq_py_type(&obj);
            assert_eq!(name, "<unknown>.<unknown>");
        });
    }
}
