//! Capacity tests for the commit-time check.
//!
//! Split from `manager_tests.rs` to stay inside the repository's 400-line
//! module limit; the split is by concern, not by size alone: these are the
//! tests for planning a *set* of names, where `manager_tests.rs` covers the
//! registry itself.
//!
//! These exercise the counting rules — dedup, already-registered names and
//! the exempt root — rather than the boundary, which the pure arithmetic
//! tests in [`super`] cover.

use std::collections::HashSet;

use pyo3::Python;
use serial_test::serial;

use super::registry::{MANAGER, ensure_registry_capacity, get_logger, new_logger_names};
use super::reset_manager;

fn names(values: &[&str]) -> HashSet<String> {
    values.iter().map(|name| (*name).to_string()).collect()
}

#[test]
fn an_empty_set_is_always_accepted() {
    ensure_registry_capacity(&names(&[])).expect("a commit of nothing cannot exceed the cap");
}

#[test]
#[serial]
fn repeats_of_one_name_count_once() {
    Python::attach(|py| {
        reset_manager();
        let _ = get_logger(py, "root").expect("root is registrable");

        // Six identifiers that resolve to one logger add one entry, so a
        // configuration listing a logger repeatedly must not be refused for
        // a cardinality it does not have.
        let staged = names(&["api", "api", "api", "api", "api", "api"]);
        ensure_registry_capacity(&staged).expect("repeats count once");
    });
}

#[test]
#[serial]
fn already_registered_names_are_not_counted_again() {
    Python::attach(|py| {
        reset_manager();
        let _ = get_logger(py, "already.here").expect("logger registered");

        ensure_registry_capacity(&names(&["already.here"]))
            .expect("re-staging an existing logger adds nothing");
    });
}

#[test]
#[serial]
fn root_is_exempt_from_the_cap() {
    Python::attach(|py| {
        reset_manager();

        // Root is both absent-and-staged and then present, and neither
        // arrangement may count towards the cap.
        ensure_registry_capacity(&names(&["root"])).expect("a staged root is exempt from the cap");
        let _ = get_logger(py, "root").expect("root is registrable");
        ensure_registry_capacity(&names(&["root"]))
            .expect("an existing root is exempt from the cap");
    });
}

#[test]
#[serial]
fn distinct_novel_names_count_once_each() {
    Python::attach(|py| {
        reset_manager();
        let _ = get_logger(py, "root").expect("root is registrable");

        let staged = names(&["a", "b.c", "b", "a"]);
        let mgr = MANAGER.read();
        let additions = new_logger_names(&mgr, staged.iter().map(String::as_str));
        assert_eq!(
            additions.len(),
            3,
            "a, b and b.c are three distinct entries, and the repeats of a \
             count once; the ancestor of b.c adds b, which the list already \
             names. Got {additions:?}",
        );
    });
}

/// A staged dotted name brings its ancestors with it, outermost first.
#[test]
#[serial]
fn an_ancestor_is_counted_before_its_descendant() {
    Python::attach(|py| {
        reset_manager();
        let _ = get_logger(py, "root").expect("root is registrable");

        let staged = names(&["api.v1.handlers"]);
        let mgr = MANAGER.read();
        let additions = new_logger_names(&mgr, staged.iter().map(String::as_str));
        assert_eq!(
            additions,
            vec!["api", "api.v1", "api.v1.handlers"],
            "the chain must be planned outermost first so each parent exists \
             before the child that names it",
        );
    });
}
