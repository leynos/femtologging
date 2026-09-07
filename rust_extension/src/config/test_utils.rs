//! Shared test utilities for configuration tests.
#![cfg(all(test, feature = "python"))]

use pyo3::Python;
use rstest::fixture;

use crate::manager;

/// Reset the logger manager before each test to ensure isolation.
#[fixture]
pub fn gil_and_clean_manager() { Python::attach(|_| manager::reset_manager()); }
