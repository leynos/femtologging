//! `PyO3` setter wrappers isolated from filter implementation.

use super::*;
use crate::macros::py_setters;

#[cfg(feature = "python")]
py_setters!(NameFilterBuilder {
    prefix: py_with_prefix => "with_prefix", String, Some,
        "Set the accepted logger-name prefix.",
});
