//! Regression tests for `disable_existing_loggers` policy.

use super::{
    ConfigBuilder, LoggerConfigBuilder, base_logger_builder, builder_with_root,
    gil_and_clean_manager, handler_count,
};
use crate::filters::{FilterBuilder, LevelFilterBuilder};
use crate::manager;
use crate::{FemtoLevel, StreamHandlerBuilder};
use pyo3::Python;
use rstest::rstest;
use serial_test::serial;

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
            .with_logger(
                "core",
                LoggerConfigBuilder::new().with_level(FemtoLevel::Info),
            )
            .build_and_init()
            .expect("initial build should succeed");

        let disable = ConfigBuilder::new()
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_root_logger(root.clone())
            .with_disable_existing_loggers(true);
        disable
            .build_and_init()
            .expect("disable pass should succeed");
        let core = manager::lookup_existing_logger(py, "core").expect("core exists");
        assert!(core.borrow(py).is_disabled(), "core should be disabled");

        let reenable = ConfigBuilder::new()
            .with_handler("h", StreamHandlerBuilder::stderr())
            .with_root_logger(root)
            .with_logger(
                "core",
                LoggerConfigBuilder::new().with_level(FemtoLevel::Debug),
            )
            .with_disable_existing_loggers(true);
        reenable
            .build_and_init()
            .expect("reenable pass should succeed");
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
            .with_logger("stale", LoggerConfigBuilder::new().with_handlers(["h"]))
            .build_and_init()
            .expect("initial build should succeed");

        let stale = manager::lookup_existing_logger(py, "stale").expect("stale exists");
        assert!(
            !stale.borrow(py).is_disabled(),
            "an explicitly named logger should start enabled",
        );

        ConfigBuilder::new()
            .with_root_logger(root.clone())
            .with_disable_existing_loggers(true)
            .build_and_init()
            .expect("disable pass should succeed");
        assert!(
            stale.borrow(py).is_disabled(),
            "the disable pass should mark stale disabled",
        );

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
        disable_a
            .build_and_init()
            .expect("disable pass should succeed");
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
