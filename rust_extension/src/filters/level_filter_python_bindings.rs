//! `PyO3` setter wrappers isolated from filter implementation.

#![expect(
    clippy::too_many_arguments,
    reason = "PyO3 generates five-argument Python call wrappers"
)]

use pyo3::prelude::*;

use super::{AsPyDict, FemtoLevel, LevelFilterBuilder};
use crate::macros::py_setters;

#[cfg(feature = "python")]
py_setters!(LevelFilterBuilder {
    max_level: py_with_max_level => "with_max_level", FemtoLevel, Some,
        "Set the maximum level permitted.",
});
