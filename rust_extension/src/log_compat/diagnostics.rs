//! Keep the Rust log bridge's own diagnostics visible without recursive dispatch.

use log::Record;

/// Return whether a target belongs to femtologging itself.
///
/// The adapter is the global Rust logger, so a record it logs about its own
/// work would be routed straight back into the adapter. The lookup-failure
/// diagnostic is the concrete case: it reports a target the registry refused,
/// which means the diagnostic describes a high-cardinality stream — and an
/// unguarded warning would add one more record per refusal to it. The tracing
/// bridge ignores its own targets for the same reason.
///
/// The `femtologging` family covers logger names this crate uses for itself;
/// the crate's own `log` targets cover module paths such as
/// `_femtologging_rs::logger::dispatcher::shard::worker`. The check compares
/// raw targets so the common module-path case allocates nothing.
pub(super) fn should_ignore_target(target: &str) -> bool {
    let is_in_family = |root: &str| {
        target
            .strip_prefix(root)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.') || rest.starts_with("::"))
    };
    is_in_family("femtologging") || is_in_family(env!("CARGO_CRATE_NAME"))
}

/// Mirror an ignored crate warning through an out-of-band writer.
///
/// For example, dispatch-capacity warnings are mirrored to stderr instead of
/// being forwarded back into the adapter that could not accept their record.
pub(super) fn mirror_ignored_warning(
    record: &Record<'_>,
    write_warning: impl FnOnce(&std::fmt::Arguments<'_>),
) {
    if should_ignore_target(record.target()) && record.level() == log::Level::Warn {
        write_warning(record.args());
    }
}
