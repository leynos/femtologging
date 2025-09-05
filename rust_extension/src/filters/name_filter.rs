//! Builder and implementation for a name-based filter.

use pyo3::prelude::*;

use crate::{
    filters::FemtoFilter,
    log_record::FemtoLogRecord,
    macros::{impl_as_pydict, py_setters, AsPyDict},
};

#[derive(Debug)]
pub struct NameFilter {
    prefix: String,
}

impl FemtoFilter for NameFilter {
    fn should_log(&self, record: &FemtoLogRecord) -> bool {
        record.logger.starts_with(&self.prefix)
    }
}

/// Builder for [`NameFilter`].
#[pyclass]
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

    fn build_inner(&self) -> Result<Self::Filter, super::FilterBuildError> {
        let prefix = self
            .prefix
            .clone()
            .ok_or_else(|| super::FilterBuildError::InvalidConfig("prefix is required".into()))?;
        Ok(NameFilter { prefix })
    }
}

impl_as_pydict!(NameFilterBuilder {
    set_opt prefix => "prefix",
});

py_setters!(NameFilterBuilder {
    prefix: py_with_prefix => "with_prefix", String, Some,
        "Set the accepted logger name prefix.",
});

fn extractor(obj: &Bound<'_, PyAny>) -> PyResult<Option<super::FilterBuilder>> {
    if let Ok(b) = obj.extract::<NameFilterBuilder>() {
        Ok(Some(super::FilterBuilder::Name(b)))
    } else {
        Ok(None)
    }
}

inventory::submit! {
    super::FilterExtractor(extractor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::FilterBuilderTrait;
    use rstest::rstest;

    fn record(name: &str) -> FemtoLogRecord {
        FemtoLogRecord::new(name, "INFO", "msg")
    }

    #[rstest]
    #[case("core", "core.child", true)]
    #[case("core", "other", false)]
    fn name_filter_behaviour(
        #[case] prefix: &str,
        #[case] logger_name: &str,
        #[case] expected: bool,
    ) {
        let builder = NameFilterBuilder::new().with_prefix(prefix);
        let filter = builder.build().expect("build should succeed");
        assert_eq!(filter.should_log(&record(logger_name)), expected);
    }
}
