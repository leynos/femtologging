//! Collection-mutation primitives shared by runtime logger builders.

use pyo3::{
    Py, PyAny, PyResult, Python,
    types::{PyDict, PyDictMethods, PyList},
};

use crate::config::types::normalize_vec;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum CollectionMutation {
    #[default]
    Unchanged,
    Replace(Vec<String>),
    Append(Vec<String>),
    Remove(Vec<String>),
    Clear,
}

impl CollectionMutation {
    pub(crate) fn replace(ids: Vec<String>) -> Self {
        Self::Replace(normalize_vec(ids))
    }

    pub(crate) fn append(ids: Vec<String>) -> Self {
        Self::Append(normalize_vec(ids))
    }

    pub(crate) fn remove(ids: Vec<String>) -> Self {
        Self::Remove(normalize_vec(ids))
    }

    pub(crate) fn apply(&self, existing: &[String]) -> Vec<String> {
        match self {
            Self::Unchanged => existing.to_vec(),
            Self::Replace(ids) => ids.clone(),
            Self::Append(ids) => {
                let mut merged = existing.to_vec();
                let mut seen = merged
                    .iter()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>();
                merged.extend(ids.iter().filter(|id| seen.insert((*id).clone())).cloned());
                merged
            }
            Self::Remove(ids) => {
                let removed = ids.iter().collect::<std::collections::BTreeSet<_>>();
                existing
                    .iter()
                    .filter(|id| !removed.contains(*id))
                    .cloned()
                    .collect()
            }
            Self::Clear => Vec::new(),
        }
    }

    pub(crate) fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = PyDict::new(py);
        let set_ids = |ids: &[String]| -> PyResult<()> {
            dict.set_item("ids", PyList::new(py, ids.iter())?)?;
            Ok(())
        };
        match self {
            Self::Unchanged => {
                dict.set_item("mode", "unchanged")?;
            }
            Self::Replace(ids) => {
                dict.set_item("mode", "replace")?;
                set_ids(ids)?;
            }
            Self::Append(ids) => {
                dict.set_item("mode", "append")?;
                set_ids(ids)?;
            }
            Self::Remove(ids) => {
                dict.set_item("mode", "remove")?;
                set_ids(ids)?;
            }
            Self::Clear => {
                dict.set_item("mode", "clear")?;
            }
        }
        Ok(dict.unbind().into())
    }
}
