//! Unit tests for the `log` crate bridge.

use log::{LevelFilter, Log};
use proptest::prelude::*;

use super::diagnostics::{LookupFailureDiagnostic, lookup_failure_message};
use super::max_level::{conservative_max_level_from, map_femto_to_log_filter};
use super::*;
use crate::handler::FemtoHandlerTrait;
use crate::logger::DEFAULT_LOGGER_LEVEL;
use crate::test_utils::collecting_handler::CollectingHandler;
use rstest::{fixture, rstest};
use serial_test::serial;
use std::sync::{
    Arc, Once,
    atomic::{AtomicUsize, Ordering},
};

static LOGGER_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[fixture]
fn unique_logger_name() -> String {
    let base = "bridge.test";
    let suffix = LOGGER_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{base}.{suffix}")
}

/// The public conversion preserves each concrete Rust facade level.
#[rstest]
#[case(log::Level::Trace, FemtoLevel::Trace)]
#[case(log::Level::Debug, FemtoLevel::Debug)]
#[case(log::Level::Info, FemtoLevel::Info)]
#[case(log::Level::Warn, FemtoLevel::Warn)]
#[case(log::Level::Error, FemtoLevel::Error)]
fn level_mapping_is_direct(#[case] level: log::Level, #[case] expected: FemtoLevel) {
    assert_eq!(FemtoLevel::from(level), expected);
}

/// Each femtologging threshold maps to the broadest Rust level it accepts.
#[rstest]
#[case::trace(FemtoLevel::Trace, log::LevelFilter::Trace)]
#[case::debug(FemtoLevel::Debug, log::LevelFilter::Debug)]
#[case::info(FemtoLevel::Info, log::LevelFilter::Info)]
#[case::warn(FemtoLevel::Warn, log::LevelFilter::Warn)]
#[case::error(FemtoLevel::Error, log::LevelFilter::Error)]
#[case::critical(FemtoLevel::Critical, log::LevelFilter::Off)]
fn femto_threshold_maps_to_its_broadest_rust_level(
    #[case] level: FemtoLevel,
    #[case] expected: log::LevelFilter,
) {
    assert_eq!(map_femto_to_log_filter(level), expected);
}

/// The facade maximum includes both the default and registered logger levels.
#[rstest]
fn conservative_maximum_includes_default_and_live_logger_levels() {
    assert_eq!(
        conservative_max_level_from(std::iter::empty()),
        log::LevelFilter::Info,
    );
    assert_eq!(
        conservative_max_level_from([FemtoLevel::Warn].into_iter()),
        log::LevelFilter::Info,
    );
    assert_eq!(
        conservative_max_level_from([FemtoLevel::Debug].into_iter()),
        log::LevelFilter::Debug,
    );
    assert_eq!(
        conservative_max_level_from([FemtoLevel::Critical].into_iter()),
        log::LevelFilter::Info,
    );
}

proptest! {
    /// Every threshold accepted by a target remains inside the facade maximum.
    #[test]
    fn conservative_maximum_covers_arbitrary_logger_thresholds(
        level_values in prop::collection::vec(0_u8..=u8::from(FemtoLevel::Critical), 0..32)
    ) {
        let levels = level_values
            .into_iter()
            .map(|value| FemtoLevel::try_from(value).expect("generated a valid FemtoLevel"))
            .collect::<Vec<_>>();
        let maximum = conservative_max_level_from(levels.iter().copied());
        let expected = levels
            .iter()
            .copied()
            .chain(std::iter::once(DEFAULT_LOGGER_LEVEL))
            .map(map_femto_to_log_filter)
            .max()
            .expect("the default logger level is always included");

        prop_assert_eq!(maximum, expected);
        prop_assert!(maximum >= map_femto_to_log_filter(DEFAULT_LOGGER_LEVEL));
        prop_assert!(levels.iter().all(|level| maximum >= map_femto_to_log_filter(*level)));
    }

    /// Recomputing after generated level changes stays conservative.
    #[test]
    fn conservative_maximum_tracks_logger_level_update_sequences(
        updates in prop::collection::vec(
            (0_usize..4, 0_u8..=u8::from(FemtoLevel::Critical)),
            0..48,
        )
    ) {
        let mut levels = [DEFAULT_LOGGER_LEVEL; 4];
        for (index, level_value) in updates {
            levels[index] = FemtoLevel::try_from(level_value)
                .expect("generated a valid FemtoLevel");
            let maximum = conservative_max_level_from(levels.iter().copied());
            let expected = levels
                .iter()
                .copied()
                .chain(std::iter::once(DEFAULT_LOGGER_LEVEL))
                .map(map_femto_to_log_filter)
                .max()
                .expect("the default logger level is always included");

            prop_assert_eq!(maximum, expected);
            let all_levels_are_accepted = levels
                .iter()
                .all(|level| maximum >= map_femto_to_log_filter(*level));
            prop_assert!(all_levels_are_accepted);
        }
    }
}

/// Set a permissive facade maximum for tests that call the adapter directly.
#[fixture]
fn log_max_level() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        log::set_max_level(LevelFilter::Trace);
    });
}

/// Dispatching a Rust record preserves its target and source metadata.
#[rstest]
#[serial]
fn adapter_dispatches_records_to_target_logger(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger
            .borrow(py)
            .add_handler(handler.clone())
            .expect("native collecting handler should register");

        let record = log::Record::builder()
            .args(format_args!("hello"))
            .level(log::Level::Info)
            .target(&logger_name)
            .module_path(Some("bridge::test"))
            .file(Some("lib.rs"))
            .line(Some(42))
            .build();

        adapter.log(&record);

        assert!(
            logger.borrow(py).flush_handlers(),
            "flush should drain the queue"
        );

        let records = handler
            .as_any()
            .downcast_ref::<CollectingHandler>()
            .expect("handler downcast")
            .collected();
        assert_eq!(records.len(), 1);
        let rec = &records[0];
        assert_eq!(rec.logger(), logger_name.as_str());
        assert_eq!(rec.level_str(), "INFO");
        assert_eq!(rec.message(), "hello");
        assert_eq!(rec.metadata().module_path, "bridge::test");
        assert_eq!(rec.metadata().filename, "lib.rs");
        assert_eq!(rec.metadata().line_number, 42);
    });
}

/// Rust module targets dispatch through their normalized dotted logger name.
#[rstest]
#[serial]
fn adapter_normalizes_rust_module_targets(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;
    let target = logger_name.replace('.', "::");

    Python::attach(|py| {
        let logger = manager::get_logger(py, &logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger
            .borrow(py)
            .add_handler(handler.clone())
            .expect("native collecting handler should register");

        let record = log::Record::builder()
            .args(format_args!("normalized"))
            .level(log::Level::Info)
            .target(&target)
            .build();

        adapter.log(&record);
        assert!(logger.borrow(py).flush_handlers());

        let records = handler
            .as_any()
            .downcast_ref::<CollectingHandler>()
            .expect("handler downcast")
            .collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].logger(), logger_name.as_str());
    });
}

/// Dispatch rechecks the target threshold before queuing a record.
#[rstest]
#[serial]
fn log_respects_logger_threshold(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;
    let logger_name = unique_logger_name;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &logger_name).expect("logger created");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        logger
            .borrow(py)
            .add_handler(handler.clone())
            .expect("native collecting handler should register");
        logger.borrow(py).set_level(FemtoLevel::Warn);

        let info_record = log::Record::builder()
            .args(format_args!("info"))
            .level(log::Level::Info)
            .target(&logger_name)
            .build();
        adapter.log(&info_record);

        let warn_record = log::Record::builder()
            .args(format_args!("warn"))
            .level(log::Level::Warn)
            .target(&logger_name)
            .build();
        adapter.log(&warn_record);

        assert!(
            logger.borrow(py).flush_handlers(),
            "flush should drain the queue"
        );

        let records = handler
            .as_any()
            .downcast_ref::<CollectingHandler>()
            .expect("handler downcast")
            .collected();
        assert_eq!(records.len(), 1, "only WARN should pass threshold");
        assert_eq!(records[0].level_str(), "WARN");
    });
}

/// Every target the bridge cannot register still reaches `root`.
///
/// A high-cardinality target stream is refused by the registry once the cap is
/// reached, and a malformed one by name validation. Both refusals travel the
/// same `Err` arm, and in both the record must survive: the bridge falls back
/// to `root` rather than dropping the record or panicking. The targets here are
/// refused by validation rather than by filling the registry, because the
/// registry is process-global and a unit test that saturated it would refuse
/// its neighbours' loggers too.
///
/// The `log_max_level` fixture is required, not decorative: `log` defaults to
/// `Off`, and the adapter returns before resolving a target below it, so
/// without the fixture this case would assert nothing.
#[rstest]
#[serial]
fn refused_targets_fall_back_to_root_without_losing_records(_log_max_level: ()) {
    manager::reset_manager();

    let refused = [
        // Longer than `MAX_LOGGER_NAME_BYTES`, so validation refuses it.
        format!(
            "stress.{}",
            "a".repeat(crate::manager::MAX_LOGGER_NAME_BYTES)
        ),
        // A newline would let a target forge a line boundary in output.
        "stress.forged\nfake [ERROR] injected".to_string(),
        // Deeper than `MAX_LOGGER_NAME_SEGMENTS`.
        ["deep"; crate::manager::MAX_LOGGER_NAME_SEGMENTS + 1].join("."),
    ];

    Python::attach(|py| {
        let adapter = FemtoLogAdapter;
        let root = manager::get_logger(py, "root").expect("root is always resolvable");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        root.borrow(py).clear_handlers();
        root.borrow(py)
            .add_handler(handler.clone())
            .expect("native collecting handler should register");

        for target in &refused {
            // `format_args!` builds a temporary `Arguments` borrow; binding it
            // first is what lets the record outlive the builder statement.
            let args = format_args!("refused {target:?}");
            let record = log::Record::builder()
                .args(args)
                .level(log::Level::Info)
                .target(target)
                .build();
            adapter.log(&record);
        }
        assert!(root.borrow(py).flush_handlers(), "flush should drain");

        let collected = handler
            .as_any()
            .downcast_ref::<CollectingHandler>()
            .expect("handler downcast")
            .collected();
        assert_eq!(
            collected.len(),
            refused.len(),
            "each refused target's record must reach root, not be dropped"
        );
        assert!(
            collected.iter().all(|record| record.logger() == "root"),
            "a refused target's record must be attributed to the fallback logger"
        );
    });

    manager::reset_manager();
}

/// A record the bridge logs about its own work does not re-enter the bridge.
///
/// The adapter is installed as the global Rust logger, so the diagnostic it
/// emits when a target is refused would be routed straight back into the
/// adapter unless its own targets are ignored. It is exactly the wrong moment
/// to be unguarded: the diagnostic fires because the caller is emitting many
/// distinct targets, so each warning would beget another record. The tracing
/// bridge already ignores its own targets; this asserts the `log` bridge does
/// too, and asserts it with a target that would otherwise resolve.
///
/// Two families must be covered, and the second is the one that bites. The
/// `femtologging` names are what this crate uses when it names a logger for
/// itself. But the warnings this crate actually emits — the shard worker's
/// "dropped N records", the handler flush failures — carry the `log` crate's
/// default target, which is the module path under the compiled crate name. A
/// guard covering only the first family would let every one of those records
/// re-enter the adapter, and the lookup-failure warning would then add a record
/// per refusal to the very stream it is diagnosing.
///
/// The `log_max_level` fixture is required: `log` defaults to `Off`, and the
/// adapter returns before consulting the ignore list below it, so without the
/// fixture the assertion would hold for the wrong reason.
#[rstest]
#[serial]
fn the_bridge_ignores_records_about_itself(_log_max_level: ()) {
    manager::reset_manager();

    Python::attach(|py| {
        let adapter = FemtoLogAdapter;
        let internal = manager::get_logger(py, "femtologging.log_compat")
            .expect("the internal logger is registrable");
        let handler = Arc::new(CollectingHandler::default()) as Arc<dyn FemtoHandlerTrait>;
        internal.borrow(py).clear_handlers();
        internal
            .borrow(py)
            .add_handler(handler.clone())
            .expect("native collecting handler should register");

        for target in [
            "femtologging",
            "femtologging.log_compat",
            "femtologging::inner",
            // The crate's own `log` target family. Every `warn!` in this crate
            // records its module path, so this is the family that carries the
            // bridge's own diagnostics — including the one this test would
            // otherwise provoke.
            env!("CARGO_CRATE_NAME"),
            concat!(
                env!("CARGO_CRATE_NAME"),
                "::logger::dispatcher::shard::worker"
            ),
        ] {
            let record = log::Record::builder()
                .args(format_args!("internal note"))
                .level(log::Level::Warn)
                .target(target)
                .build();
            adapter.log(&record);
        }

        assert!(
            internal.borrow(py).flush_handlers(),
            "flush should drain the internal logger"
        );
        assert!(
            handler
                .as_any()
                .downcast_ref::<CollectingHandler>()
                .expect("handler downcast")
                .collected()
                .is_empty(),
            "a femtologging-owned target must not be forwarded back through the bridge"
        );
    });

    manager::reset_manager();
}

/// Refusing a target quotes it back bounded and escaped.
///
/// A refused target is caller-controlled and has not passed name validation —
/// that is why it was refused — so the diagnostic must bound what it writes.
/// Two megabyte-long targets, one of them carrying a newline and a shard worker
/// module path, would otherwise put several megabytes on stderr in a single
/// line, and the newline would forge a record boundary in the output the name
/// rules exist to protect.
///
/// The message is asserted directly rather than by capturing stderr: the
/// warner is a process-wide static shared with every other test in the binary,
/// so whether a given call warns depends on what ran before it. Splitting the
/// message out of the warner makes the property under test — the bound and the
/// escaping — independent of that scheduling.
#[rstest]
fn refused_target_diagnostics_are_bounded_and_escaped() {
    let escaped = format!("stress.forged\nfake [ERROR] {}", "b".repeat(1_000_000));
    let overlong = format!("stress.{}", "c".repeat(1_000_000));

    let message = lookup_failure_message(LookupFailureDiagnostic {
        reason: "invalid logger target",
        target: &escaped,
        normalized: &escaped,
        detail: "logger name cannot contain the control character '\\n'",
        count: 1,
    });

    assert!(
        !message.contains('\n'),
        "the diagnostic must not carry a raw newline from the target: {message:?}",
    );
    assert!(
        message.contains('\u{2026}'),
        "a truncated target must be marked as truncated, got {message}",
    );
    assert!(
        message.len() < 1_000,
        "a one-megabyte target must not put its size on stderr, got {} bytes",
        message.len(),
    );
    for target in [&escaped, &overlong] {
        let quoted = manager::describe_name(target);
        assert!(
            !quoted.contains('\n') && quoted.len() <= 64 + "\u{2026}".len() + 2,
            "a refused target must be quoted bounded and escaped, got {quoted:?}",
        );
    }
}

/// The family check matches a root exactly or at a separator, nothing looser.
///
/// The guard decides whether a record is dropped before it is ever resolved,
/// so a check that matched too much would silently swallow real log records.
/// `femtologgingx` and `femtologging-extra` are different targets that merely
/// share a prefix; only a separator or the end of the target may follow the
/// family root. Both spellings are pinned because `log` targets arrive in both.
#[rstest]
#[case("femtologging", true)]
#[case("femtologging.log_compat", true)]
#[case("femtologging::inner", true)]
#[case("femtologgingx", false)]
#[case("femtologging-extra", false)]
#[case("my.femtologging.job", false)]
#[case("application", false)]
fn the_family_check_requires_a_separator_after_the_root(
    #[case] target: &str,
    #[case] should_ignore: bool,
) {
    assert_eq!(
        should_ignore_target(target),
        should_ignore,
        "target {target:?} must be {} by the bridge's own-target guard",
        if should_ignore {
            "ignored"
        } else {
            "forwarded"
        },
    );
}

/// Enablement follows the live level of a registered target logger.
#[rstest]
#[serial]
fn enabled_uses_the_registered_target_threshold(_log_max_level: (), unique_logger_name: String) {
    let adapter = FemtoLogAdapter;

    Python::attach(|py| {
        let logger = manager::get_logger(py, &unique_logger_name).expect("logger created");
        logger.borrow(py).set_level(FemtoLevel::Warn);
    });

    for (level, expected) in [
        (log::Level::Trace, false),
        (log::Level::Debug, false),
        (log::Level::Info, false),
        (log::Level::Warn, true),
        (log::Level::Error, true),
    ] {
        let metadata = Metadata::builder()
            .level(level)
            .target(&unique_logger_name)
            .build();
        assert_eq!(adapter.enabled(&metadata), expected, "level {level}");
    }
}

/// Enablement normalizes module targets without creating unknown loggers.
#[rstest]
#[serial]
fn enabled_normalizes_module_targets_and_leaves_unknown_targets_unregistered(
    _log_max_level: (),
    unique_logger_name: String,
) {
    let adapter = FemtoLogAdapter;
    let rust_target = unique_logger_name.replace('.', "::");

    Python::attach(|py| {
        let logger = manager::get_logger(py, &unique_logger_name).expect("logger created");
        logger.borrow(py).set_level(FemtoLevel::Warn);
    });

    let normalized_metadata = Metadata::builder()
        .level(log::Level::Info)
        .target(&rust_target)
        .build();
    assert!(!adapter.enabled(&normalized_metadata));

    let unknown_target = format!("{unique_logger_name}.missing");
    assert_eq!(manager::registered_logger_level(&unknown_target), None);
    let unknown_metadata = Metadata::builder()
        .level(log::Level::Info)
        .target(&unknown_target)
        .build();
    assert!(adapter.enabled(&unknown_metadata));
    assert_eq!(manager::registered_logger_level(&unknown_target), None);
}

/// Invalid targets use the root threshold, or the default when root is absent.
#[rstest]
#[serial]
fn enabled_uses_root_for_invalid_targets_and_default_when_root_is_absent(_log_max_level: ()) {
    let adapter = FemtoLogAdapter;
    manager::reset_manager();

    let invalid_info = Metadata::builder()
        .level(log::Level::Info)
        .target("invalid..target")
        .build();
    assert!(adapter.enabled(&invalid_info));
    assert_eq!(manager::registered_logger_level("root"), None);

    Python::attach(|py| {
        let root = manager::get_logger(py, "root").expect("root logger created");
        root.borrow(py).set_level(FemtoLevel::Warn);
    });

    let invalid_debug = Metadata::builder()
        .level(log::Level::Debug)
        .target("invalid..target")
        .build();
    let invalid_warn = Metadata::builder()
        .level(log::Level::Warn)
        .target("invalid..target")
        .build();
    assert!(!adapter.enabled(&invalid_debug));
    assert!(!adapter.enabled(&invalid_info));
    assert!(adapter.enabled(&invalid_warn));
    assert_eq!(
        manager::registered_logger_level("root"),
        Some(FemtoLevel::Warn)
    );
}
