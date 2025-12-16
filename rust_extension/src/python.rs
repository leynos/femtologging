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
            let code = CString::new("class Foo: pass\n").expect("valid Python code");
            let filename = CString::new("mymod.py").expect("valid filename");
            let module_name = CString::new("mymod").expect("valid module name");
            let module = PyModule::from_code(
                py,
                code.as_c_str(),
                filename.as_c_str(),
                module_name.as_c_str(),
            )
            .expect("module creation succeeds");
            let obj = module
                .getattr("Foo")
                .expect("Foo class exists")
                .call0()
                .expect("Foo() call succeeds");
            let name = fq_py_type(&obj);
            assert_eq!(name, "mymod.Foo");
        });
    }

    #[test]
    fn falls_back_when_attrs_missing() {
        Python::with_gil(|py| {
            let code = CString::new(concat!(
                "class Meta(type):\n",
                "    def __getattribute__(cls, name):\n",
                "        if name in ('__module__', '__qualname__'):\n",
                "            raise AttributeError(name)\n",
                "        return super().__getattribute__(name)\n",
                "\n",
                "class Bar(metaclass=Meta):\n",
                "    pass\n",
            ))
            .expect("valid Python code");
            let filename = CString::new("mymod.py").expect("valid filename");
            let module_name = CString::new("mymod").expect("valid module name");
            let module = PyModule::from_code(
                py,
                code.as_c_str(),
                filename.as_c_str(),
                module_name.as_c_str(),
            )
            .expect("module creation succeeds");
            let obj = module
                .getattr("Bar")
                .expect("Bar class exists")
                .call0()
                .expect("Bar() call succeeds");
            let name = fq_py_type(&obj);
            assert_eq!(name, "<unknown>.<unknown>");
        });
    }
}
