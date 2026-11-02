//! Tests for warning visibility at the Rust log bridge boundary.
//!
//! The adapter is the global Rust logger, so its own diagnostics must not
//! re-enter it, and a target it refuses must still be reported without
//! letting a caller-controlled string loose on stderr.

use std::sync::Arc;

use log::Log;
use pyo3::prelude::*;
use rstest::rstest;
use serial_test::serial;

use super::FemtoLogAdapter;
use super::diagnostics::{
    LookupFailureDiagnostic, lookup_failure_message, mirror_ignored_warning, should_ignore_target,
};
use super::test_support::log_max_level;
use crate::handler::FemtoHandlerTrait;
use crate::manager;
use crate::test_utils::collecting_handler::CollectingHandler;

#[test]
fn ignored_internal_warnings_are_mirrored_to_stderr() {
    let target = format!("{}::logger::producer::admission", env!("CARGO_CRATE_NAME"));
    let warning = log::Record::builder()
        .args(format_args!("dispatch capacity exhausted"))
        .level(log::Level::Warn)
        .target(&target)
        .build();
    let internal_info = log::Record::builder()
        .args(format_args!("ordinary internal record"))
        .level(log::Level::Info)
        .target(&target)
        .build();
    let application_warning = log::Record::builder()
        .args(format_args!("application warning"))
        .level(log::Level::Warn)
        .target("application")
        .build();
    let mut diagnostics = Vec::new();

    mirror_ignored_warning(&warning, |message| {
        diagnostics.push(message.to_string());
    });
    mirror_ignored_warning(&internal_info, |message| {
        diagnostics.push(message.to_string());
    });
    mirror_ignored_warning(&application_warning, |message| {
        diagnostics.push(message.to_string());
    });

    assert_eq!(diagnostics, ["dispatch capacity exhausted"]);
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
