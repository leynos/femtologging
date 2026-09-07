//! `PyO3` setter wrappers isolated from filter implementation.

#![expect(
    clippy::too_many_arguments,
    reason = "PyO3 generates five-argument Python call wrappers"
)]

use pyo3::prelude::*;

use super::{AsPyDict, NameFilterBuilder};
use crate::macros::py_setters;

#[cfg(feature = "python")]
py_setters!(NameFilterBuilder {
    prefix: py_with_prefix => "with_prefix", String, Some,
        "Set the accepted logger-name prefix.",
});
