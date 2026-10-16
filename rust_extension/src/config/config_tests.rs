//! Unit tests for configuration builders.
#![cfg(all(test, feature = "python"))]

use super::test_utils::gil_and_clean_manager;
use super::*;
use crate::config::ConfigError;
use crate::filters::{FilterBuilder, LevelFilterBuilder};
use crate::manager;
use crate::{FemtoLevel, StreamHandlerBuilder};
use pyo3::Python;
use rstest::{fixture, rstest};
use serial_test::serial;
use std::sync::Arc;

fn builder_with_root(root: LoggerConfigBuilder) -> ConfigBuilder {
    ConfigBuilder::new()
        .with_handler("h", StreamHandlerBuilder::stderr())
        .with_root_logger(root)
}

#[fixture]
fn base_logger_builder() -> (ConfigBuilder, LoggerConfigBuilder) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = builder_with_root(root.clone());
    (builder, root)
}

/// Fetch the logger named `name` and report its handler count.
///
/// Fallible so callers can `.expect()` at the assertion site, keeping the
/// panic line attached to the calling test.
fn handler_count(py: Python<'_>, name: &str) -> pyo3::PyResult<usize> {
    let logger = manager::lookup_existing_logger(py, name)?;
    let count = logger.borrow(py).handlers_for_test().len();
    Ok(count)
}

/// Assert that the logger named `$name` has `$expected` handlers, panicking
/// with `$reason` from the caller's line on mismatch.
macro_rules! assert_handler_count {
    ($py:expr, $name:expr, $expected:expr, $reason:expr) => {{
        let count = handler_count($py, $name).expect("existing logger lookup should succeed");
        assert_eq!(count, $expected, "{}", $reason);
    }};
}

#[rstest]
fn build_rejects_invalid_version() {
    let builder = ConfigBuilder::new().with_version(2);
    let err = builder
        .build_and_init()
        .expect_err("version 2 should be rejected");
    assert!(matches!(err, ConfigError::UnsupportedVersion(2)));
}

#[rstest]
fn build_rejects_missing_root() {
    let builder = ConfigBuilder::new();
    let err = builder
        .build_and_init()
        .expect_err("root logger is required");
    assert!(matches!(err, ConfigError::MissingRootLogger));
}

#[rstest]
#[serial]
fn build_accepts_default_version(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    let (_, root) = base_logger_builder;
    let builder = ConfigBuilder::new().with_root_logger(root);
    assert!(builder.build_and_init().is_ok());
}

#[rstest]
#[serial]
fn shared_handler_attached_once(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::attach(|py| {
        let logger_cfg = LoggerConfigBuilder::new().with_handlers(["h"]);
        let (builder, _root) = base_logger_builder;
        let builder = builder
            .with_logger("first", logger_cfg.clone())
            .with_logger("second", logger_cfg);
        builder.build_and_init().expect("build should succeed");
        let first = manager::get_logger(py, "first").expect("get_logger('first') should succeed");
        let second =
            manager::get_logger(py, "second").expect("get_logger('second') should succeed");
        let h1 = first.borrow(py).handlers_for_test();
        let h2 = second.borrow(py).handlers_for_test();
        assert!(
            Arc::ptr_eq(&h1[0], &h2[0]),
            "handler Arc pointers should be shared"
        );
    });
}

#[rstest]
#[case::handler(
    "handler",
    |b: ConfigBuilder| b.with_handler("exists", StreamHandlerBuilder::stderr()),
    LoggerConfigBuilder::new().with_handlers(["missing"]),
)]
#[case::filter(
    "filter",
    |b: ConfigBuilder| {
        b.with_filter(
            "exists",
            FilterBuilder::Level(LevelFilterBuilder::new().with_max_level(FemtoLevel::Info)),
        )
    },
    LoggerConfigBuilder::new().with_filters(["missing"]),
)]
#[serial]
fn unknown_id_rejected(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
    #[case] _kind: &str,
    #[case] add: fn(ConfigBuilder) -> ConfigBuilder,
    #[case] cfg: LoggerConfigBuilder,
) {
    let (_, root) = base_logger_builder;
    let builder = add(ConfigBuilder::new()
        .with_root_logger(root)
        .with_logger("child", cfg));
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should fail for unknown id");
    if let ConfigError::UnknownIds(mut ids) = err {
        ids.sort();
        assert_eq!(ids, vec!["missing".to_string()]);
    } else {
        panic!("expected UnknownIds");
    }
}

#[rstest]
#[serial]
fn reconfig_with_unknown_filter_preserves_existing_filters(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::attach(|py| {
        let (_, root) = base_logger_builder;
        let filt = LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug);
        let builder = ConfigBuilder::new()
            .with_filter("lvl", FilterBuilder::Level(filt))
            .with_root_logger(root.clone())
            .with_logger("core", LoggerConfigBuilder::new().with_filters(["lvl"]));
        builder
            .build_and_init()
            .expect("initial build should succeed");
        let logger = manager::get_logger(py, "core").expect("get_logger('core') should succeed");
        assert!(logger.borrow(py).log(FemtoLevel::Error, "drop").is_none());
        let bad = ConfigBuilder::new()
            .with_root_logger(root)
            .with_logger("core", LoggerConfigBuilder::new().with_filters(["missing"]));
        assert!(bad.build_and_init().is_err());
        assert!(
            logger
                .borrow(py)
                .log(FemtoLevel::Error, "still drop")
                .is_none()
        );
    });
}

#[rstest]
#[serial]
fn unknown_filter_id_rejected(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    let (_, root) = base_logger_builder;
    let logger_cfg = LoggerConfigBuilder::new().with_filters(["missing"]);
    let builder = ConfigBuilder::new()
        .with_root_logger(root)
        .with_logger("child", logger_cfg);
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should fail for unknown filter id");
    assert!(matches!(err, ConfigError::UnknownIds(ids) if ids == vec!["missing".to_string()]));
}

#[rstest]
#[case::handler(
    "handler",
    |b: ConfigBuilder| b.with_handler("exists", StreamHandlerBuilder::stderr()),
    LoggerConfigBuilder::new().with_handlers(["missing1", "missing2"]),
)]
#[case::filter(
    "filter",
    |b: ConfigBuilder| {
        b.with_filter(
            "exists",
            FilterBuilder::Level(LevelFilterBuilder::new().with_max_level(FemtoLevel::Info)),
        )
    },
    LoggerConfigBuilder::new().with_filters(["missing1", "missing2"]),
)]
#[serial]
fn multiple_unknown_ids_rejected(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
    #[case] _kind: &str,
    #[case] add: fn(ConfigBuilder) -> ConfigBuilder,
    #[case] cfg: LoggerConfigBuilder,
) {
    let (_, root) = base_logger_builder;
    let builder = add(ConfigBuilder::new()
        .with_root_logger(root)
        .with_logger("child", cfg));
    let err = builder.build_and_init().expect_err("should fail");
    if let ConfigError::UnknownIds(mut ids) = err {
        ids.sort();
        assert_eq!(ids, vec!["missing1".to_string(), "missing2".to_string()]);
    } else {
        panic!("expected UnknownIds");
    }
}

// Duplicate handler/filter IDs are deduplicated by `LoggerConfigBuilder`'s
// public constructors, so configurations cannot produce duplicate ID errors
// through the supported API surface.
#[rstest]
#[serial]
fn disable_existing_loggers_clears_unmentioned(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::attach(|py| {
        let (builder, root) = base_logger_builder;
        let filt = LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug);
        let builder = builder
            .with_filter("f", FilterBuilder::Level(filt))
            .with_logger(
                "stale",
                LoggerConfigBuilder::new()
                    .with_handlers(["h"])
                    .with_filters(["f"]),
            );
        builder
            .build_and_init()
            .expect("initial build should succeed");

        assert_handler_count!(py, "stale", 1, "stale logger should start active");

        let rebuild = ConfigBuilder::new()
            .with_root_logger(root)
            .with_disable_existing_loggers(true);
        rebuild.build_and_init().expect("rebuild should succeed");

        assert_handler_count!(py, "stale", 0, "stale logger should be disabled");
        let stale = manager::lookup_existing_logger(py, "stale").expect("stale logger exists");
        assert!(
            stale.borrow(py).is_disabled(),
            "an omitted logger must be flagged disabled, not merely stripped",
        );
    });
}

#[rstest]
#[serial]
fn disabled_logger_rejects_records_and_reports_disabled(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::attach(|py| {
        let (builder, root) = base_logger_builder;
        // The omitted logger keeps the default `propagate = true`, so only the
        // disabled flag can stop its records reaching an ancestor handler.
        builder
            .with_logger(
                "omitted",
                LoggerConfigBuilder::new().with_level(FemtoLevel::Info),
            )
            .build_and_init()
            .expect("initial build should succeed");
        let omitted = manager::lookup_existing_logger(py, "omitted").expect("omitted exists");
        assert!(
            omitted.borrow(py).propagate(),
            "the omitted logger must start with default propagation enabled",
        );

        let rebuild = ConfigBuilder::new()
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_root_logger(root)
            .with_disable_existing_loggers(true);
        rebuild.build_and_init().expect("rebuild should succeed");

        assert!(
            omitted.borrow(py).is_disabled(),
            "omitted logger should be disabled",
        );
        assert!(
            !omitted.borrow(py).is_enabled_for(FemtoLevel::Error),
            "a disabled logger must report every level as disabled",
        );
        assert_eq!(
            omitted.borrow(py).log(FemtoLevel::Error, "must not escape"),
            None,
            "a disabled logger must reject the record without dispatching it",
        );
        let root_logger = manager::lookup_existing_logger(py, "root").expect("root exists");
        assert!(
            !root_logger.borrow(py).is_disabled(),
            "the root logger must never be disabled",
        );
    });
}

#[rstest]
#[serial]
fn explicitly_named_logger_is_reenabled(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::attach(|py| {
        let (builder, root) = base_logger_builder;
        builder
            .with_logger("core", LoggerConfigBuilder::new().with_level(FemtoLevel::Info))
            .build_and_init()
            .expect("initial build should succeed");

        let disable = ConfigBuilder::new()
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_root_logger(root.clone())
            .with_disable_existing_loggers(true);
        disable.build_and_init().expect("disable pass should succeed");
        let core = manager::lookup_existing_logger(py, "core").expect("core exists");
        assert!(core.borrow(py).is_disabled(), "core should be disabled");

        let reenable = ConfigBuilder::new()
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_root_logger(root)
            .with_logger("core", LoggerConfigBuilder::new().with_level(FemtoLevel::Debug))
            .with_disable_existing_loggers(true);
        reenable.build_and_init().expect("reenable pass should succeed");
        assert!(
            !core.borrow(py).is_disabled(),
            "a later configuration that names the logger must re-enable it",
        );
        assert!(
            core.borrow(py).is_enabled_for(FemtoLevel::Debug),
            "the re-enabled logger should honour its configured level",
        );
    });
}

#[rstest]
#[serial]
fn disabling_leaves_unmentioned_logger_state_when_flag_is_false(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::attach(|py| {
        let (builder, root) = base_logger_builder;
        builder
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_root_logger(LoggerConfigBuilder::new())
            .with_logger("stale", LoggerConfigBuilder::new().with_handlers(["h"]))
            .with_disable_existing_loggers(true)
            .build_and_init()
            .expect("initial build should succeed");

        let stale = manager::lookup_existing_logger(py, "stale").expect("stale exists");
        assert!(stale.borrow(py).is_disabled(), "stale should start disabled");

        let rebuild = ConfigBuilder::new()
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_root_logger(root);
        rebuild.build_and_init().expect("rebuild should succeed");

        assert!(
            stale.borrow(py).is_disabled(),
            "with the flag false an earlier disabled state must be preserved",
        );
    });
}

#[rstest]
#[serial]
fn ancestor_of_configured_descendant_keeps_its_disabled_state(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::attach(|py| {
        let (builder, root) = base_logger_builder;
        builder
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_logger("a", LoggerConfigBuilder::new().with_level(FemtoLevel::Info))
            .build_and_init()
            .expect("initial build should succeed");

        let disable_a = ConfigBuilder::new()
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_root_logger(root.clone())
            .with_disable_existing_loggers(true);
        disable_a.build_and_init().expect("disable pass should succeed");
        let a = manager::lookup_existing_logger(py, "a").expect("a exists");
        assert!(a.borrow(py).is_disabled(), "a should be disabled");

        // `a` is in the keep set as an ancestor of `a.b`, but it was not named
        // explicitly, so it must stay disabled rather than being re-enabled.
        let configure_child = ConfigBuilder::new()
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_root_logger(root)
            .with_logger(
                "a.b",
                LoggerConfigBuilder::new().with_level(FemtoLevel::Info),
            )
            .with_disable_existing_loggers(true);
        configure_child
            .build_and_init()
            .expect("descendant configuration should succeed");

        assert!(
            a.borrow(py).is_disabled(),
            "a kept ancestor must not be re-enabled merely because it is kept",
        );
        let child = manager::lookup_existing_logger(py, "a.b").expect("a.b exists");
        assert!(
            !child.borrow(py).is_disabled(),
            "the explicitly named descendant must be enabled",
        );
    });
}

#[rstest]
#[serial]
fn failed_reconfiguration_preserves_unmentioned_loggers(_gil_and_clean_manager: ()) {
    Python::attach(|py| {
        let initial = builder_with_root(LoggerConfigBuilder::new())
            .with_logger("stale", LoggerConfigBuilder::new().with_handlers(["h"]));
        initial
            .build_and_init()
            .expect("initial configuration should succeed");
        assert_handler_count!(py, "stale", 1, "stale logger should start active");

        let invalid = ConfigBuilder::new()
            .with_root_logger(LoggerConfigBuilder::new())
            .with_disable_existing_loggers(true)
            .with_logger(
                "broken",
                LoggerConfigBuilder::new().with_handlers(["missing"]),
            );
        invalid
            .build_and_init()
            .expect_err("unknown handler must reject the replacement configuration");

        let runtime = manager::snapshot_runtime_state();
        let stale = runtime
            .logger_states
            .get("stale")
            .expect("failed replacement must preserve stale attachment metadata");
        assert_eq!(
            stale.handler_ids(),
            &["h".to_string()],
            "failed replacement must not replace runtime attachment metadata",
        );
        assert_handler_count!(
            py,
            "stale",
            1,
            "failed replacement must leave unmentioned loggers unchanged"
        );
    });
}
#[rstest(
    ancestor_names,
    case::parent(&["parent"]),
    case::grandparent(&["grandparent", "grandparent.parent"]),
)]
#[serial]
fn disable_existing_loggers_keeps_ancestors(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
    ancestor_names: &[&str],
) {
    Python::attach(|py| {
        let (mut builder, root) = base_logger_builder;
        for name in ancestor_names {
            builder = builder.with_logger(*name, LoggerConfigBuilder::new().with_handlers(["h"]));
        }
        builder
            .build_and_init()
            .expect("initial build should succeed");

        for name in ancestor_names {
            assert_handler_count!(py, name, 1, "ancestor logger should start active");
        }

        let child_name = format!(
            "{}.child",
            ancestor_names.last().expect("at least one ancestor")
        );
        let rebuild = builder_with_root(root)
            .with_logger(&child_name, LoggerConfigBuilder::new().with_handlers(["h"]))
            .with_disable_existing_loggers(true);
        rebuild.build_and_init().expect("rebuild should succeed");

        for name in ancestor_names {
            assert_handler_count!(py, name, 1, "ancestor logger should remain active");
        }
        assert_handler_count!(py, &child_name, 1, "child logger should retain its handler");
        // femtologging does not mutate Python's standard `logging` module state.
    });
}

#[rstest]
#[serial]
fn default_level_configures_root_when_missing_level(_gil_and_clean_manager: ()) {
    Python::attach(|py| {
        let builder = ConfigBuilder::new()
            .with_handler("stderr", StreamHandlerBuilder::stderr())
            .with_default_level(FemtoLevel::Warn)
            .with_root_logger(LoggerConfigBuilder::new().with_handlers(["stderr"]));
        builder
            .build_and_init()
            .expect("build should apply default root level");

        let root = manager::get_logger(py, "root").expect("root logger should exist");
        assert!(
            root.borrow(py)
                .log(FemtoLevel::Info, "suppressed")
                .is_none(),
            "root should honour the configured default WARN level",
        );
        assert!(
            root.borrow(py).log(FemtoLevel::Error, "emitted").is_some(),
            "records at or above WARN should be emitted",
        );
    });
}

#[rstest]
#[serial]
fn default_level_applies_to_child_loggers(_gil_and_clean_manager: ()) {
    Python::attach(|py| {
        let child_cfg = LoggerConfigBuilder::new().with_handlers(["console"]);
        let builder = ConfigBuilder::new()
            .with_handler("console", StreamHandlerBuilder::stderr())
            .with_default_level(FemtoLevel::Info)
            .with_root_logger(
                LoggerConfigBuilder::new()
                    .with_level(FemtoLevel::Warn)
                    .with_handlers(["console"]),
            )
            .with_logger("worker", child_cfg);
        builder
            .build_and_init()
            .expect("build should succeed with default levels");

        let worker = manager::get_logger(py, "worker").expect("worker logger should exist");
        assert!(
            worker
                .borrow(py)
                .log(FemtoLevel::Debug, "suppressed")
                .is_none(),
            "child logger should inherit the default INFO level",
        );
        assert!(
            worker.borrow(py).log(FemtoLevel::Info, "visible").is_some(),
            "records meeting the default level should be emitted",
        );
    });
}

/// A complete snapshot of everything a failed configuration must preserve.
///
/// Handler and filter identity are compared by pointer so a re-created but
/// equivalent attachment still counts as a change.
#[derive(Debug, PartialEq, Eq)]
struct LoggerStateSnapshot {
    handler_ptrs: Vec<usize>,
    filter_ptrs: Vec<usize>,
    level: String,
    propagate: bool,
    disabled: bool,
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
        handler_ptrs: logger_ref.handler_ptrs_for_test(),
        filter_ptrs: logger_ref.filter_ptrs_for_test(),
        level: logger_ref.level(),
        propagate: logger_ref.propagate(),
        disabled: logger_ref.is_disabled(),
        attachment,
    })
}

/// Names covered by the transactional-failure comparisons.
///
/// `keep` is a live logger an accepted configuration would re-plan, `stale` is
/// one a previous configuration disabled, and `root` must survive every
/// rejection.
const TRANSACTION_LOGGER_NAMES: [&str; 3] = ["root", "keep", "stale"];

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

    let invalid = mutate(transactional_builder(root).with_disable_existing_loggers(true));
    invalid
        .build_and_init()
        .expect_err("the invalid configuration must be rejected");

    let after = TRANSACTION_LOGGER_NAMES
        .iter()
        .map(|name| snapshot_logger_state(py, name).expect("logger survives a rejection"))
        .collect::<Vec<_>>();
    assert_eq!(
        after, before,
        "a rejected configuration must leave every pre-existing logger unchanged",
    );
}

#[rstest]
#[serial]
fn later_unknown_handler_id_rolls_back_every_logger(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::attach(|py| {
        let (_, root) = base_logger_builder;
        assert_rejected_configuration_changes_nothing(py, root, |builder| {
            // The unknown id is on the last logger, after the plan for `keep`
            // has already been prepared and any eager commit would have run.
            builder.with_logger(
                "zzz",
                LoggerConfigBuilder::new().with_handlers(["missing"]),
            )
        });
    });
}

#[rstest]
#[serial]
fn later_unknown_filter_id_rolls_back_every_logger(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::attach(|py| {
        let (_, root) = base_logger_builder;
        assert_rejected_configuration_changes_nothing(py, root, |builder| {
            builder.with_logger(
                "zzz",
                LoggerConfigBuilder::new().with_filters(["missing"]),
            )
        });
    });
}

#[rstest]
#[serial]
fn invalid_logger_name_rolls_back_every_logger(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::attach(|py| {
        let (_, root) = base_logger_builder;
        assert_rejected_configuration_changes_nothing(py, root, |builder| {
            builder.with_logger("a..b", LoggerConfigBuilder::new())
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
            .with_logger("stale", LoggerConfigBuilder::new().with_level(FemtoLevel::Info))
            .build_and_init()
            .expect("initial build should succeed");

        let disable = ConfigBuilder::new()
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_root_logger(root)
            .with_disable_existing_loggers(true);
        disable.build_and_init().expect("disable pass should succeed");
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
