//! Compile-time checks for the Rust `log` bridge's level mappings.
//!
//! The mappers are `const fn` so a caller can build a threshold or a filter at
//! compile time. These items fail the build if one stops being const-evaluable,
//! which a run-time test cannot detect: a `const fn` that quietly loses its
//! qualifier still passes every behavioural test.
//!
//! The mappers are crate-private, so an external `trybuild` fixture cannot
//! name them — a UI fixture compiles as its own crate. Each `const _`
//! initialiser below is therefore the enforcing test for one guarantee, in the
//! same spirit as `crate::http_handler::transport_origin_tests`. The bridge's
//! public counterpart, `crate::tracing_compat::layer`, is also pinned from
//! outside by `tests/ui/pass/tracing_layer_const.rs`.

use log::LevelFilter;

use super::max_level::map_femto_to_log_filter;
use super::{map_femto_to_log_level, map_log_level};
use crate::FemtoLevel;

// `map_log_level` maps a `log` facade level into femtologging's level type.
const _: FemtoLevel = map_log_level(log::Level::Info);
// `map_femto_to_log_level` maps in the other direction.
const _: log::Level = map_femto_to_log_level(FemtoLevel::Critical);
// `map_femto_to_log_filter` builds a `log` threshold from a level.
const _: LevelFilter = map_femto_to_log_filter(FemtoLevel::Critical);
