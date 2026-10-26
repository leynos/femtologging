//! Regression tests for transactional configuration rejection.

use super::{ConfigBuilder, LoggerConfigBuilder, base_logger_builder, gil_and_clean_manager};
use crate::filters::{FilterBuilder, LevelFilterBuilder};
use crate::manager;
use crate::{FemtoLevel, StreamHandlerBuilder};
use pyo3::Python;
use rstest::rstest;
use serial_test::serial;
use std::sync::Arc;

/// A complete snapshot of everything a failed configuration must preserve.
///
/// Handler and filter identity are compared by pointer so a re-created but
/// equivalent attachment still counts as a change.
#[derive(Debug, PartialEq, Eq)]
struct LoggerStateSnapshot {
    /// Pointer identities of the logger's attached handlers, in order.
    handler_ptrs: Vec<usize>,
    /// Pointer identities of the logger's attached filters, in order.
    filter_ptrs: Vec<usize>,
    /// Current minimum log level.
    level: String,
    /// Whether this logger propagates records to its parent.
    propagate: bool,
    /// Whether this logger rejects records as disabled.
    disabled: bool,
    /// Handler and filter IDs recorded by the runtime configuration snapshot.
    attachment: Option<(Vec<String>, Vec<String>)>,
}

fn snapshot_logger_state(py: Python<'_>, name: &str) -> pyo3::PyResult<LoggerStateSnapshot> {
    let logger = manager::lookup_existing_logger(py, name)?;
    let logger_ref = logger.borrow(py);
    let attachment = manager::snapshot_runtime_state()
        .logger_states
        .get(name)
        .map(|state| (state.handler_ids().to_vec(), state.filter_ids().to_vec()));
    Ok(LoggerStateSnapshot {
        handler_ptrs: logger_ref.handler_ptrs_for_test_native(),
        filter_ptrs: logger_ref.filter_ptrs_for_test_native(),
        level: logger_ref.level(),
        propagate: logger_ref.propagate(),
        disabled: logger_ref.is_disabled(),
        attachment,
    })
}

/// Snapshot attachment registries and ID metadata published by configuration.
#[derive(Debug, PartialEq, Eq)]
struct RuntimeAttachmentSnapshot {
    /// Handler registry IDs paired with the pointer identities they resolve to.
    handlers: Vec<(String, usize)>,
    /// Handler registry IDs paired with their configured filter IDs.
    handler_filter_ids: Vec<(String, Vec<String>)>,
    /// Filter registry IDs paired with the pointer identities they resolve to.
    filters: Vec<(String, usize)>,
    /// Logger names paired with their configured handler and filter IDs.
    logger_states: Vec<(String, Vec<String>, Vec<String>)>,
}

fn snapshot_runtime_attachments() -> RuntimeAttachmentSnapshot {
    let runtime = manager::snapshot_runtime_state();
    RuntimeAttachmentSnapshot {
        handlers: runtime
            .handler_registry
            .iter()
            .map(|(id, handler)| (id.clone(), Arc::as_ptr(handler) as *const () as usize))
            .collect(),
        handler_filter_ids: runtime
            .handler_filter_ids
            .iter()
            .map(|(id, filter_ids)| (id.clone(), filter_ids.clone()))
            .collect(),
        filters: runtime
            .filter_registry
            .iter()
            .map(|(id, filter)| (id.clone(), Arc::as_ptr(filter) as *const () as usize))
            .collect(),
        logger_states: runtime
            .logger_states
            .iter()
            .map(|(name, state)| {
                (
                    name.clone(),
                    state.handler_ids().to_vec(),
                    state.filter_ids().to_vec(),
                )
            })
            .collect(),
    }
}

/// Names covered by the transactional-failure comparisons.
///
/// `keep` is a live logger an accepted configuration would re-plan, `stale` is
/// one a previous configuration disabled, and `root` must survive every
/// rejection.
const TRANSACTION_LOGGER_NAMES: [&str; 3] = ["root", "keep", "stale"];

/// Invalid logger-plan changes exercised by the rollback regression.
#[derive(Clone, Copy, Debug)]
enum InvalidLoggerConfiguration {
    UnknownHandlerId,
    UnknownFilterId,
    InvalidLoggerName,
}

impl InvalidLoggerConfiguration {
    /// Add this invalid logger plan after the valid plans in `builder`.
    fn append_to(self, builder: ConfigBuilder) -> ConfigBuilder {
        match self {
            Self::UnknownHandlerId => {
                builder.with_logger("zzz", LoggerConfigBuilder::new().with_handlers(["missing"]))
            }
            Self::UnknownFilterId => {
                builder.with_logger("zzz", LoggerConfigBuilder::new().with_filters(["missing"]))
            }
            Self::InvalidLoggerName => builder.with_logger("a..b", LoggerConfigBuilder::new()),
        }
    }
}

/// Return a builder carrying the shared handler and filter definitions.
fn transactional_builder(root: LoggerConfigBuilder) -> ConfigBuilder {
    ConfigBuilder::new()
        .with_handler("h", StreamHandlerBuilder::stderr())
        .with_filter(
            "f",
            FilterBuilder::Level(LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug)),
        )
        .with_root_logger(root)
}

/// Reject a configuration and assert no pre-existing logger changed.
///
/// `mutate` supplies the invalid element, so one body covers unknown handler
/// ids, unknown filter ids, and malformed logger names. The comparison covers
/// handler and filter identity, level, propagation, the disabled flag, and
/// runtime attachment metadata, and it includes a logger that an earlier
/// configuration already disabled.
fn assert_rejected_configuration_changes_nothing(
    py: Python<'_>,
    root: LoggerConfigBuilder,
    mutate: impl FnOnce(ConfigBuilder) -> ConfigBuilder,
) {
    // Stage one configures `keep` and `stale`; stage two omits `stale` while
    // disabling, so the comparison below starts from a state that already has
    // one disabled logger. A rejected run must neither re-enable it nor leave
    // any other logger partially disabled.
    transactional_builder(root.clone())
        .with_logger(
            "keep",
            LoggerConfigBuilder::new()
                .with_handlers(["h"])
                .with_filters(["f"]),
        )
        .with_logger("stale", LoggerConfigBuilder::new().with_handlers(["h"]))
        .build_and_init()
        .expect("stage one should succeed");
    transactional_builder(root.clone())
        .with_logger(
            "keep",
            LoggerConfigBuilder::new()
                .with_handlers(["h"])
                .with_filters(["f"]),
        )
        .with_disable_existing_loggers(true)
        .build_and_init()
        .expect("stage two should succeed");

    let stale = manager::lookup_existing_logger(py, "stale").expect("stale exists");
    assert!(
        stale.borrow(py).is_disabled(),
        "the baseline must include a logger a previous configuration disabled",
    );

    let before = TRANSACTION_LOGGER_NAMES
        .iter()
        .map(|name| snapshot_logger_state(py, name).expect("baseline logger exists"))
        .collect::<Vec<_>>();
    let registered_before = manager::registered_logger_names_for_test();
    let runtime_before = snapshot_runtime_attachments();

    let invalid = mutate(transactional_builder(root).with_disable_existing_loggers(true));
    invalid
        .build_and_init()
        .expect_err("the invalid configuration must be rejected");

    let after = TRANSACTION_LOGGER_NAMES
        .iter()
        .map(|name| snapshot_logger_state(py, name).expect("logger survives a rejection"))
        .collect::<Vec<_>>();
    let registered_after = manager::registered_logger_names_for_test();
    let runtime_after = snapshot_runtime_attachments();
    assert_eq!(
        after, before,
        "a rejected configuration must leave every pre-existing logger unchanged",
    );
    assert_eq!(
        registered_after, registered_before,
        "a rejected configuration must not change manager registry membership",
    );
    assert_eq!(
        runtime_after, runtime_before,
        "a rejected configuration must preserve runtime handler and filter metadata",
    );
}

#[rstest]
#[case::unknown_handler_id(InvalidLoggerConfiguration::UnknownHandlerId)]
#[case::unknown_filter_id(InvalidLoggerConfiguration::UnknownFilterId)]
#[case::invalid_logger_name(InvalidLoggerConfiguration::InvalidLoggerName)]
#[serial]
fn invalid_logger_configuration_rolls_back_every_logger(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
    #[case] invalid_configuration: InvalidLoggerConfiguration,
) {
    Python::attach(|py| {
        let (_, root) = base_logger_builder;
        assert_rejected_configuration_changes_nothing(py, root, |builder| {
            invalid_configuration.append_to(builder)
        });
    });
}

#[rstest]
#[serial]
fn omitted_logger_that_was_already_disabled_is_not_disturbed(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::attach(|py| {
        let (builder, root) = base_logger_builder;
        builder
            .with_logger(
                "stale",
                LoggerConfigBuilder::new().with_level(FemtoLevel::Info),
            )
            .build_and_init()
            .expect("initial build should succeed");

        let disable = ConfigBuilder::new()
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_root_logger(root)
            .with_disable_existing_loggers(true);
        disable
            .build_and_init()
            .expect("disable pass should succeed");
        let stale = manager::lookup_existing_logger(py, "stale").expect("stale exists");
        assert!(stale.borrow(py).is_disabled(), "stale should be disabled");

        // Re-running the same configuration must be idempotent: a repeated
        // reconfiguration keeps the disabled state rather than toggling it.
        let repeat = ConfigBuilder::new()
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_root_logger(LoggerConfigBuilder::new().with_handlers(["h"]))
            .with_disable_existing_loggers(true);
        repeat.build_and_init().expect("repeat should succeed");
        assert!(
            stale.borrow(py).is_disabled(),
            "repeated reconfiguration must keep the logger disabled",
        );
    });
}
