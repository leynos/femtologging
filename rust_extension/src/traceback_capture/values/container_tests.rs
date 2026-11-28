//! Container rendering at the item, depth and byte ceilings.

use pyo3::types::{PyFrozenSet, PyList, PySet, PyTuple};

use super::*;

#[test]
fn a_marker_separator_fits_inside_the_ceiling() {
    Python::attach(|py| {
        let list = PyList::new(py, ["12345", "6"]).expect("list should build");
        let rendered = render(list.as_any(), 8, 0).expect("a list should render");

        assert!(
            rendered.text.len() <= 8,
            "the marker separator must be part of the ceiling: {:?}",
            rendered.text
        );
        assert!(rendered.clipped, "the byte ceiling caused the cut");
    });
}

#[test]
fn a_complete_container_is_not_reported_as_clipped() {
    Python::attach(|py| {
        let list = PyList::new(py, [1, 2]).expect("list should build");
        let rendered = render(list.as_any(), 32, 0).expect("a list should render");

        assert_eq!(rendered.text, "[1, 2]");
        assert!(!rendered.truncated);
        assert!(!rendered.clipped);
    });
}

#[test]
fn a_small_complete_container_fits_before_the_marker_is_reserved() {
    Python::attach(|py| {
        let list = PyList::new(py, [1]).expect("list should build");
        let rendered = render(list.as_any(), 7, 0).expect("a list should render");

        assert_eq!(rendered.text, "[1]");
        assert!(!rendered.truncated);
    });
}

#[test]
fn a_tiny_ceiling_still_bounds_the_marker() {
    Python::attach(|py| {
        let list = PyList::new(py, [123456]).expect("list should build");
        let rendered = render(list.as_any(), 5, 0).expect("a list should render");

        assert!(
            rendered.text.len() <= 5,
            "the summary must respect the ceiling: {:?}",
            rendered.text
        );
        assert!(rendered.clipped);
    });
}

#[test]
fn singleton_tuples_and_empty_sets_keep_their_forms() {
    Python::attach(|py| {
        let tuple = PyTuple::new(py, [1]).expect("tuple should build");
        assert_eq!(render(tuple.as_any(), 16, 0).unwrap().text, "(1,)");

        let set = PySet::empty(py).expect("set should build");
        assert_eq!(render(set.as_any(), 16, 0).unwrap().text, "set()");

        let frozen = PyFrozenSet::empty(py).expect("frozenset should build");
        assert_eq!(render(frozen.as_any(), 16, 0).unwrap().text, "frozenset()");
    });
}

#[test]
fn a_set_subclass_keeps_its_type_wrapper() {
    Python::attach(|py| {
        let value = py
            .eval(c"type('Tags', (set,), {})({1})", None, None)
            .expect("set subclass should build");
        let rendered = render(&value, 32, 0).expect("a set subclass should render");

        assert_eq!(rendered.text, "Tags({1})");
        assert!(!rendered.truncated);
    });
}
