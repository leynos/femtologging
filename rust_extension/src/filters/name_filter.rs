//! Builder and implementation for a name-based filter.

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
use crate::macros::{AsPyDict, impl_as_pydict, py_setters};
use crate::{
    filters::{FemtoFilter, FilterBuildError},
    log_record::FemtoLogRecord,
};

#[derive(Debug)]
pub struct NameFilter {
    prefix: String,
    prefix_dot: String,
}

impl FemtoFilter for NameFilter {
    fn should_log(&self, record: &FemtoLogRecord) -> bool {
        record.logger == self.prefix || record.logger.starts_with(&self.prefix_dot)
    }
}

/// Builder for [`NameFilter`].
#[cfg_attr(feature = "python", pyclass)]
#[derive(Clone, Debug, Default)]
pub struct NameFilterBuilder {
    prefix: Option<String>,
}

impl NameFilterBuilder {
    /// Create a new `NameFilterBuilder`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the prefix that logger names must start with.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }
}

impl super::FilterBuilderTrait for NameFilterBuilder {
    type Filter = NameFilter;

    fn build_inner(&self) -> Result<Self::Filter, FilterBuildError> {
        let prefix = self
            .prefix
            .clone()
            .ok_or_else(|| FilterBuildError::InvalidConfig("prefix is required".into()))?;
        if prefix.is_empty() {
            return Err(FilterBuildError::InvalidConfig(
                "prefix must not be empty".into(),
            ));
        }
        let prefix_dot = format!("{prefix}.");
        Ok(NameFilter { prefix, prefix_dot })
    }
}

#[cfg(feature = "python")]
impl_as_pydict!(NameFilterBuilder {
    set_opt prefix => "prefix",
});

#[cfg(feature = "python")]
py_setters!(NameFilterBuilder {
    prefix: py_with_prefix => "with_prefix", String, Some,
        "Set the accepted logger-name prefix.",
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::{FilterBuildError, FilterBuilderTrait};
    use rstest::rstest;

    fn record(name: &str) -> FemtoLogRecord {
        FemtoLogRecord::new(name, "INFO", "msg")
    }

    #[rstest]
    #[case("core", "core.child", true)]
    #[case("core", "other", false)]
    #[case("core", "corey", false)]
    fn name_filter_behaviour(
        #[case] prefix: &str,
        #[case] logger_name: &str,
        #[case] expected: bool,
    ) {
        let builder = NameFilterBuilder::new().with_prefix(prefix);
        let filter = builder.build().expect("build should succeed");
        assert_eq!(filter.should_log(&record(logger_name)), expected);
    }
    #[test]
    fn empty_prefix_rejected() {
        let builder = NameFilterBuilder::new().with_prefix("");
        let err = builder.build().err().expect("build should fail");
        assert!(matches!(err, FilterBuildError::InvalidConfig(_)));
    }
}
