//! Timed-rotation parameters shared by the handler and its Python bindings.
//!
//! What stayed in this module after the rotation mechanics moved out is the
//! constructor bundle the public handler is built from: the validated schedule
//! and the retention count. The handler wrapper lives in [`super::handler`],
//! the rotation itself and its unwinding in [`super::rotation_strategy`], the
//! rollover deadline in [`super::rollover_schedule`], and backup pruning in
//! [`super::backup_retention`].

use super::schedule::TimedRotationSchedule;

/// Bundles timed-rotation parameters passed to the handler constructor.
pub(crate) struct TimedRotationConfig {
    /// Validated schedule used by the worker before each record write.
    pub(crate) schedule: TimedRotationSchedule,
    /// Number of timestamped backups retained by the timed handler.
    pub(crate) backup_count: usize,
}
