//! Builder and implementation for a level-based filter.

use log::warn;
use pyo3::prelude::*;

use crate::{
    filters::FemtoFilter,
    level::FemtoLevel,
    log_record::FemtoLogRecord,
    macros::{impl_as_pydict, py_setters, AsPyDict},
};

#[derive(Debug)]
pub struct LevelFilter {
    max_level: FemtoLevel,
}

impl FemtoFilter for LevelFilter {
    fn should_log(&self, record: &FemtoLogRecord) -> bool {
        let record_level: FemtoLevel = match record.level.parse::<FemtoLevel>() {
            Ok(level) => level,
            Err(e) => {
                warn!(
                    concat!(
                        "FemtoLog: Failed to parse log level '{}' for record from ",
                        "logger '{}'. Error: {:?}. Filtering out this record."
                    ),
                    record.level, record.logger, e
                );
                return false;
            }
        };
        record_level <= self.max_level
    }
}

/// Builder for [`LevelFilter`].
#[pyclass]
#[derive(Clone, Debug, Default)]
pub struct LevelFilterBuilder {
    max_level: Option<FemtoLevel>,
}

impl LevelFilterBuilder {
    /// Create a new `LevelFilterBuilder`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum level allowed.
    pub fn with_max_level(mut self, level: FemtoLevel) -> Self {
        self.max_level = Some(level);
        self
    }
}

impl super::FilterBuilderTrait for LevelFilterBuilder {
    type Filter = LevelFilter;

    fn build_inner(&self) -> Result<Self::Filter, super::FilterBuildError> {
        let lvl = self.max_level.ok_or_else(|| {
            super::FilterBuildError::InvalidConfig("max_level is required".into())
        })?;
        Ok(LevelFilter { max_level: lvl })
    }
}

impl_as_pydict!(LevelFilterBuilder {
    set_opt_to_string max_level => "max_level",
});

py_setters!(LevelFilterBuilder {
    max_level: py_with_max_level => "with_max_level", FemtoLevel, Some,
        "Set the maximum level permitted.",
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::FilterBuilderTrait;
    use rstest::rstest;

    fn record(level: FemtoLevel) -> FemtoLogRecord {
        FemtoLogRecord::new("core", &level.to_string(), "msg")
    }

    #[rstest]
    #[case(FemtoLevel::Info, FemtoLevel::Info, true)]
    #[case(FemtoLevel::Info, FemtoLevel::Error, false)]
    fn level_filter_behaviour(
        #[case] max: FemtoLevel,
        #[case] rec_level: FemtoLevel,
        #[case] expected: bool,
    ) {
        let builder = LevelFilterBuilder::new().with_max_level(max);
        let filter = builder.build().expect("build should succeed");
        assert_eq!(filter.should_log(&record(rec_level)), expected);
    }
}
