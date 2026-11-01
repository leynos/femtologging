//! Production-path coverage for bridge enablement after configuration.
//!
//! The unit tests exercise the conservative maximum arithmetic and the
//! adapter's lookup in isolation. These tests install the bridge first and
//! then publish thresholds through the real
//! [`ConfigBuilder::build_and_init`](_femtologging_rs::ConfigBuilder::build_and_init)
//! path, which is the ordering an application sees. A reset case proves the
//! threshold index does not keep a removed logger's level.

#![cfg(all(feature = "log-compat", feature = "test-util"))]

use _femtologging_rs::{
    ConfigBuilder, FemtoLevel, LoggerConfigBuilder, StreamHandlerBuilder, install_global_logger,
    manager,
};
use pyo3::Python;
use serial_test::serial;

/// The target configured only after the bridge is installed.
const LATE_TARGET: &str = "bridge.config.late_target";

/// The target configured after a reset to widen the facade maximum.
const WITNESS_TARGET: &str = "bridge.config.witness";

/// The root threshold used by every case.
///
/// It stays at [`FemtoLevel::Info`] so a configured target's level is the only
/// reason the threshold widens.
const ROOT_LEVEL: FemtoLevel = FemtoLevel::Info;

/// Install the bridge as the process-wide Rust logger.
///
/// The integration test binary is a fresh process, so it owns the global
/// logger slot; a failure here means something else in the harness installed a
/// logger, which would invalidate the assertions below.
fn install_bridge_for_test() -> bool {
    Python::attach(|_| manager::reset_manager());
    install_global_logger()
}

/// Configure one non-root logger at `level` through production configuration.
fn configure_logger(name: &str, level: FemtoLevel) {
    ConfigBuilder::new()
        .with_handler("sink", StreamHandlerBuilder::stderr())
        .with_root_logger(LoggerConfigBuilder::new().with_level(ROOT_LEVEL))
        .with_logger(
            name,
            LoggerConfigBuilder::new()
                .with_handlers(["sink"])
                .with_level(level),
        )
        .build_and_init()
        .expect("configuration should commit the named logger");
}

/// A target unknown at install time becomes enabled once configuration adds it.
///
/// Installs the bridge while `LATE_TARGET` is unregistered, so the bridge's
/// conservative maximum is only the default threshold and a `Debug` record for
/// that target is refused. `build_and_init` then registers the target at
/// `Debug`; both the facade maximum and the target check must admit it without
/// a second installation.
#[test]
#[serial]
fn configuring_a_new_target_publishes_its_threshold_to_the_bridge() {
    assert!(
        install_bridge_for_test(),
        "the integration test process should install the bridge"
    );

    assert_eq!(
        log::max_level(),
        log::LevelFilter::Info,
        "an unregistered target leaves only the default threshold"
    );
    assert!(
        !log::log_enabled!(target: LATE_TARGET, log::Level::Debug),
        "an unregistered target must follow the default threshold"
    );

    configure_logger(LATE_TARGET, FemtoLevel::Debug);

    assert!(
        log::max_level() >= log::LevelFilter::Debug,
        "configuration must widen the facade maximum to the new target's level"
    );
    assert!(
        log::log_enabled!(target: LATE_TARGET, log::Level::Debug),
        "the newly configured target must accept Debug records"
    );
}

/// A reset removes the dropped logger's threshold from the target index.
///
/// `WITNESS_TARGET` keeps the facade maximum at `Trace` after the reset, so a
/// `Trace` check for the removed `LATE_TARGET` cannot be answered by the
/// process-wide prefilter alone: only the target index can reject it. If the
/// index retained the removed logger's threshold, that check would pass.
#[test]
#[serial]
fn resetting_the_manager_drops_removed_logger_thresholds() {
    assert!(
        install_bridge_for_test(),
        "the integration test process should install the bridge"
    );

    configure_logger(LATE_TARGET, FemtoLevel::Trace);
    assert!(
        log::log_enabled!(target: LATE_TARGET, log::Level::Trace),
        "the configured target must accept Trace records before the reset"
    );

    Python::attach(|_| manager::reset_manager());
    configure_logger(WITNESS_TARGET, FemtoLevel::Trace);

    assert_eq!(
        log::max_level(),
        log::LevelFilter::Trace,
        "the witness target must keep the facade maximum permissive"
    );
    assert!(
        log::log_enabled!(target: WITNESS_TARGET, log::Level::Trace),
        "the witness target must accept Trace records"
    );
    assert!(
        !log::log_enabled!(target: LATE_TARGET, log::Level::Trace),
        "a removed target must fall back instead of keeping its threshold"
    );
}
