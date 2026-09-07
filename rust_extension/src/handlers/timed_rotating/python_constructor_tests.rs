//! Python API regression tests for `TimedHandlerOptions` construction.

use pyo3::{
    Bound, Python,
    exceptions::PyTypeError,
    prelude::*,
    types::{PyAny, PyAnyMethods, PyDict, PyDictMethods},
};

use super::TimedHandlerOptions;

#[derive(Debug, PartialEq, Eq)]
struct OptionValues {
    capacity: usize,
    flush_interval: isize,
    policy: String,
    when: String,
    interval: u32,
    backup_count: usize,
    utc: bool,
    at_time: Option<String>,
}

fn read_option_values(options: &Bound<'_, PyAny>) -> PyResult<OptionValues> {
    Ok(OptionValues {
        capacity: options.getattr("capacity")?.extract()?,
        flush_interval: options.getattr("flush_interval")?.extract()?,
        policy: options.getattr("policy")?.extract()?,
        when: options.getattr("when")?.extract()?,
        interval: options.getattr("interval")?.extract()?,
        backup_count: options.getattr("backup_count")?.extract()?,
        utc: options.getattr("utc")?.extract()?,
        at_time: options.getattr("at_time")?.extract()?,
    })
}

fn expected_values() -> OptionValues {
    OptionValues {
        capacity: 64,
        flush_interval: 2,
        policy: "block".to_owned(),
        when: "D".to_owned(),
        interval: 3,
        backup_count: 4,
        utc: true,
        at_time: Some("12:34:56".to_owned()),
    }
}

fn python_time(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("datetime")?.getattr("time")?.call1((12, 34, 56))
}

#[test]
fn python_constructor_uses_documented_defaults() {
    Python::attach(|py| {
        let options = py
            .get_type::<TimedHandlerOptions>()
            .call0()
            .expect("Python constructor must accept no arguments");

        let values = read_option_values(&options).expect("Python properties must be readable");
        assert_eq!(values.capacity, 1024);
        assert_eq!(values.flush_interval, 1);
        assert_eq!(values.policy, "drop");
        assert_eq!(values.when, "H");
        assert_eq!(values.interval, 1);
        assert_eq!(values.backup_count, 0);
        assert!(!values.utc);
        assert_eq!(values.at_time, None);
    });
}

#[test]
fn python_constructor_accepts_all_positional_arguments() {
    Python::attach(|py| {
        let time = python_time(py).expect("datetime.time must construct a valid value");
        let options = py
            .get_type::<TimedHandlerOptions>()
            .call1((64_usize, 2_isize, "block", "D", 3_u32, 4_usize, true, time))
            .expect("Python constructor must accept the complete positional form");

        assert_eq!(
            read_option_values(&options).expect("Python properties must be readable"),
            expected_values()
        );
    });
}

#[test]
fn python_constructor_accepts_all_keyword_arguments() {
    Python::attach(|py| {
        let kwargs = PyDict::new(py);
        kwargs
            .set_item("capacity", 64_usize)
            .expect("capacity must be set");
        kwargs
            .set_item("flush_interval", 2_isize)
            .expect("flush interval must be set");
        kwargs
            .set_item("policy", "block")
            .expect("policy must be set");
        kwargs.set_item("when", "D").expect("when must be set");
        kwargs
            .set_item("interval", 3_u32)
            .expect("interval must be set");
        kwargs
            .set_item("backup_count", 4_usize)
            .expect("backup count must be set");
        kwargs.set_item("utc", true).expect("UTC flag must be set");
        kwargs
            .set_item(
                "at_time",
                python_time(py).expect("datetime.time must construct a valid value"),
            )
            .expect("time must be set");

        let options = py
            .get_type::<TimedHandlerOptions>()
            .call((), Some(&kwargs))
            .expect("Python constructor must accept the complete keyword form");

        assert_eq!(
            read_option_values(&options).expect("Python properties must be readable"),
            expected_values()
        );
    });
}

#[test]
fn python_constructor_rejects_duplicate_and_unknown_keywords() {
    Python::attach(|py| {
        let class = py.get_type::<TimedHandlerOptions>();
        let duplicate = PyDict::new(py);
        duplicate
            .set_item("capacity", 65_usize)
            .expect("capacity must be set");
        let duplicate_error = class
            .call((64_usize,), Some(&duplicate))
            .expect_err("duplicate positional and keyword arguments must fail");
        assert!(
            duplicate_error.is_instance_of::<PyTypeError>(py),
            "duplicate arguments must raise TypeError"
        );

        let unknown = PyDict::new(py);
        unknown
            .set_item("unexpected", true)
            .expect("unknown key must be set");
        let unknown_error = class
            .call((), Some(&unknown))
            .expect_err("unknown keyword arguments must fail");
        assert!(
            unknown_error.is_instance_of::<PyTypeError>(py),
            "unknown keyword arguments must raise TypeError"
        );
    });
}

#[test]
fn python_constructor_accepts_none_for_optional_at_time() {
    Python::attach(|py| {
        let kwargs = PyDict::new(py);
        kwargs
            .set_item("at_time", py.None())
            .expect("None must be set");
        let options = py
            .get_type::<TimedHandlerOptions>()
            .call((), Some(&kwargs))
            .expect("None must be accepted for optional at_time");

        let values = read_option_values(&options).expect("Python properties must be readable");
        assert_eq!(values.at_time, None);
    });
}

#[test]
fn python_constructor_preserves_type_errors() {
    Python::attach(|py| {
        let class = py.get_type::<TimedHandlerOptions>();
        let invalid_capacity = PyDict::new(py);
        invalid_capacity
            .set_item("capacity", "not-a-number")
            .expect("invalid capacity must be set");
        let capacity_error = class
            .call((), Some(&invalid_capacity))
            .expect_err("invalid capacity type must fail");
        assert!(
            capacity_error.is_instance_of::<PyTypeError>(py),
            "invalid capacity type must raise TypeError"
        );

        let invalid_time = PyDict::new(py);
        invalid_time
            .set_item("at_time", "not-a-time")
            .expect("invalid time must be set");
        let time_error = class
            .call((), Some(&invalid_time))
            .expect_err("invalid at_time type must fail");
        assert!(
            time_error.is_instance_of::<PyTypeError>(py),
            "invalid at_time type must raise TypeError"
        );
    });
}
