//! Compile-time checks for the Rust `log` bridge's level mappings.
//!
//! The mappers are `const fn` so a caller can build a threshold or a filter at
//! compile time. These items fail the build if one stops being const-evaluable,
//! which a run-time test cannot detect.

use log::LevelFilter;

use super::max_level::map_femto_to_log_filter;
use super::{map_femto_to_log_level, map_log_level};
use crate::FemtoLevel;

const _: FemtoLevel = map_log_level(log::Level::Info);
const _: log::Level = map_femto_to_log_level(FemtoLevel::Critical);
const _: LevelFilter = map_femto_to_log_filter(FemtoLevel::Critical);
