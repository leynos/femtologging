//! Fixtures shared by the `log` bridge's unit test modules.
//!
//! The bridge is covered by several sibling test modules, and each needs the
//! same two facilities: a logger name no other test in the binary already
//! holds, and a facade maximum permissive enough for a direct
//! [`log::Log::log`] call to reach the adapter.
//!
//! Both live in [`crate::test_utils::logging_test_helpers`] so the Python
//! convenience tests can share them too. They are re-exported here under the
//! path the bridge tests already import.

pub(super) use crate::test_utils::logging_test_helpers::{log_max_level, unique_logger_name};
