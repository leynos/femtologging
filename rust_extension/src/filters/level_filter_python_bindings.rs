//! `PyO3` setter wrappers isolated from filter implementation.

use super::*;
use crate::macros::py_setters;

#[cfg(feature = "python")]
py_setters!(LevelFilterBuilder {
    max_level: py_with_max_level => "with_max_level", FemtoLevel, Some,
        "Set the maximum level permitted.",
});
