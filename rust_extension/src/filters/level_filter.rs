//! Builder and implementation for a level-based filter.

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
use crate::macros::{AsPyDict, impl_as_pydict, py_setters};
use crate::{filters::FemtoFilter, level::FemtoLevel, log_record::FemtoLogRecord};

#[derive(Debug)]
pub struct LevelFilter {
    max_level: FemtoLevel,
}

impl FemtoFilter for LevelFilter {
    fn should_log(&self, record: &FemtoLogRecord) -> bool {
        match record.parsed_level {
            Some(level) => level <= self.max_level,
            None => false,
        }
    }
}

/// Builder for [`LevelFilter`].
#[cfg_attr(feature = "python", pyclass)]
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
    ///
    /// When called from Python, `level` may be a `FemtoLevel` instance or a recognised level string.
    pub fn with_max_level(mut self, level: FemtoLevel) -> Self {
        self.max_level = Some(level);
        self
    }
}

impl super::FilterBuilderTrait for LevelFilterBuilder {
    type Filter = LevelFilter;

    fn build_inner(&self) -> Result<Self::Filter, super::FilterBuildError> {
        let lvl = self.max_level.ok_or_else(|| {
            super::FilterBuildError::InvalidConfig(
                "max_level is required; expected one of TRACE|DEBUG|INFO|WARN|ERROR|CRITICAL"
                    .into(),
            )
        })?;
        Ok(LevelFilter { max_level: lvl })
    }
}

#[cfg(feature = "python")]
impl_as_pydict!(LevelFilterBuilder {
    set_opt_to_string max_level => "max_level",
});

#[cfg(feature = "python")]
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
        FemtoLogRecord::new("core", level, "msg")
    }

    #[rstest]
    #[case(FemtoLevel::Info, FemtoLevel::Info, true)]
    #[case(FemtoLevel::Info, FemtoLevel::Error, false)]
    #[case(FemtoLevel::Error, FemtoLevel::Critical, false)]
    #[case(FemtoLevel::Error, FemtoLevel::Warn, true)]
    fn level_filter_behaviour(
        #[case] max: FemtoLevel,
        #[case] rec_level: FemtoLevel,
        #[case] expected: bool,
    ) {
        let builder = LevelFilterBuilder::new().with_max_level(max);
        let filter = builder.build().expect("build should succeed");
        assert_eq!(filter.should_log(&record(rec_level)), expected);
    }

    #[test]
    fn rejects_unparseable_level() {
        use crate::log_record::RecordMetadata;

        let filter = LevelFilterBuilder::new()
            .with_max_level(FemtoLevel::Info)
            .build()
            .expect("build should succeed");
        // Simulate a record with an invalid/missing parsed level
        let record = FemtoLogRecord {
            logger: "core".to_owned(),
            level: "NOPE".to_owned(),
            parsed_level: None,
            message: "msg".to_owned(),
            metadata: RecordMetadata::default(),
            exception_payload: None,
            stack_payload: None,
        };
        assert!(!filter.should_log(&record));
    }
}
