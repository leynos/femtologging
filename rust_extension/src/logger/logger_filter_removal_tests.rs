//! Filter-detachment behaviour for [`FemtoLogger`](crate::FemtoLogger).
//!
//! Removing a filter by identity and clearing the whole chain are the two
//! ways a logger stops applying a filter. Both are asserted through the
//! record's fate rather than through the filter list, because the list is
//! private and the record is what a caller observes.

use super::*;
use crate::filters::{FilterBuilderTrait, LevelFilterBuilder, NameFilterBuilder};

#[test]
fn removing_and_clearing_filters() {
    let logger = FemtoLogger::new("remove".into()).expect("test logger should be constructible");
    let filt = LevelFilterBuilder::new()
        .with_max_level(FemtoLevel::Info)
        .build()
        .expect("build should succeed");
    logger.add_filter(filt.clone());
    assert!(logger.log(FemtoLevel::Error, "msg").is_none());
    assert!(logger.remove_filter(&filt));
    assert!(logger.log(FemtoLevel::Error, "msg").is_some());

    let f1 = LevelFilterBuilder::new()
        .with_max_level(FemtoLevel::Error)
        .build()
        .expect("build should succeed");
    let f2 = NameFilterBuilder::new()
        .with_prefix("other")
        .build()
        .expect("build should succeed");
    logger.add_filter(f1);
    logger.add_filter(f2);
    assert!(logger.log(FemtoLevel::Error, "msg").is_none());
    logger.clear_filters();
    assert!(logger.log(FemtoLevel::Error, "msg").is_some());
}

#[test]
fn removing_unknown_filter_returns_false() {
    let logger = FemtoLogger::new("unknown".into()).expect("test logger should be constructible");
    let filt = LevelFilterBuilder::new()
        .with_max_level(FemtoLevel::Info)
        .build()
        .expect("build should succeed");
    assert!(!logger.remove_filter(&filt));
    assert!(logger.log(FemtoLevel::Error, "err").is_some());
}
