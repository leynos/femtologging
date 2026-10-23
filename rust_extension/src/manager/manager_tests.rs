//! Tests for the global logger registry.
//!
//! These cover the read-only lookup path, the cardinality bound, and the `log`
//! bridge's flush sweep. The cap is asserted through the registry's own
//! arithmetic rather than by building ten thousand loggers: the boundary is
//! what matters, and materialising the population would cost ten thousand live
//! loggers — which, before the shared dispatch pool lands, is one thread each.
//!
//! The module is compiled in every feature lane so the `log-compat` flush
//! sweep keeps its coverage; individual tests carry their own feature gate.

use pyo3::Python;
use serial_test::serial;

use super::registry::MANAGER;
use super::{MAX_LOGGER_NAME_BYTES, MAX_LOGGERS, get_logger, reset_manager, validate_logger_name};

#[test]
#[serial]
fn get_logger_rejects_an_invalid_name_without_registering_anything() {
    Python::attach(|py| {
        reset_manager();

        assert!(
            get_logger(py, "").is_err(),
            "an empty name should be refused before any insertion",
        );
        assert!(
            get_logger(py, "bad\nname").is_err(),
            "a name carrying a control character should be refused",
        );
        assert_eq!(
            super::registry::named_logger_count_for_test(),
            0,
            "a refused name must not leave a logger behind",
        );
    });
}

#[test]
#[serial]
fn get_logger_validates_before_creating_root() {
    Python::attach(|py| {
        reset_manager();

        // Root is created lazily by `get_logger`, so a refused call must not
        // be the thing that brings it into existence.
        assert!(get_logger(py, "invalid name with spaces\u{0}").is_err());
        assert!(
            MANAGER.read().loggers.is_empty(),
            "validation failure must not create the root logger",
        );
    });
}

#[test]
#[serial]
fn the_cap_does_not_count_root_but_does_count_named_loggers() {
    use super::registry::{named_logger_count_for_test, registry_has_room_for_test};

    Python::attach(|py| {
        reset_manager();

        assert_eq!(named_logger_count_for_test(), 0);
        assert!(registry_has_room_for_test(), "an empty registry has room");

        let _ = get_logger(py, "root").expect("root is always registrable");
        assert_eq!(
            named_logger_count_for_test(),
            0,
            "root is exempt from the cap, so it is not counted",
        );

        let _ = get_logger(py, "named").expect("the first named logger fits");
        assert_eq!(
            named_logger_count_for_test(),
            1,
            "a named logger counts against the cap",
        );
    });
}

#[test]
fn registry_has_room_admits_the_last_free_slot_only() {
    // The boundary as pure arithmetic: `registry_has_room` is the single
    // predicate every insertion path consults, so it can be checked exactly
    // without populating a registry.
    assert!(
        !super::registry::registry_has_room(MAX_LOGGERS),
        "the registry must refuse a new name once it holds MAX_LOGGERS",
    );
    assert!(
        super::registry::registry_has_room(MAX_LOGGERS - 1),
        "the last free slot must still admit a name",
    );
}

#[test]
fn validate_logger_name_is_reachable_from_the_manager_path() {
    // `FemtoLogger::new` and every registry insertion path call this, so the
    // manager path is the surface Python sees.
    assert!(validate_logger_name("api.v1").is_ok());
    assert!(validate_logger_name(&"a".repeat(MAX_LOGGER_NAME_BYTES + 1)).is_err());
}

#[test]
#[serial]
fn a_rejected_name_leaves_existing_loggers_untouched() {
    Python::attach(|py| {
        reset_manager();

        let logger = get_logger(py, "keep.me").expect("a valid logger is registered");
        let before = MANAGER.read().loggers.len();

        assert!(get_logger(py, "keep\u{7f}me").is_err());
        assert_eq!(
            MANAGER.read().loggers.len(),
            before,
            "a refused name must not change the registry size",
        );

        // The identity of an already-registered logger survives every refusal.
        let again = get_logger(py, "keep.me").expect("the logger is still there");
        assert!(
            logger.is(&again),
            "a rejected sibling name must not displace an existing logger",
        );
    });
}

#[cfg(feature = "python")]
mod lookup {
    //! Tests for read-only logger lookup.

    use pyo3::Python;
    use serial_test::serial;

    use super::super::registry::MANAGER;
    use super::super::{get_logger, lookup_existing_logger, reset_manager};

    #[test]
    #[serial]
    fn lookup_existing_logger_does_not_create_missing_loggers() {
        Python::attach(|py| {
            reset_manager();

            assert!(
                lookup_existing_logger(py, "missing").is_err(),
                "a missing logger should return an explicit lookup error",
            );
            assert!(
                MANAGER.read().loggers.is_empty(),
                "a failed lookup must not create root or the requested logger",
            );

            assert!(
                get_logger(py, "existing").is_ok(),
                "logger setup should succeed before the lookup",
            );
            assert!(
                lookup_existing_logger(py, "existing").is_ok(),
                "an existing logger should be returned by the read-only lookup",
            );
        });
    }
}

#[cfg(feature = "python")]
mod capacity {
    //! Tests for the commit-time capacity check.
    //!
    //! These exercise the counting rules — dedup, already-registered names and
    //! the exempt root — rather than the boundary, which the pure arithmetic
    //! test above covers.

    use std::collections::HashSet;

    use pyo3::Python;
    use serial_test::serial;

    use super::super::registry::{MANAGER, ensure_registry_capacity, get_logger, new_logger_names};
    use super::super::reset_manager;

    fn names(values: &[&str]) -> HashSet<String> {
        values.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn an_empty_set_is_always_accepted() {
        ensure_registry_capacity(&names(&[])).expect("a commit of nothing cannot exceed the cap");
    }

    #[test]
    #[serial]
    fn repeats_of_one_name_count_once() {
        Python::attach(|py| {
            reset_manager();
            let _ = get_logger(py, "root").expect("root is registrable");

            // Six identifiers that resolve to one logger add one entry, so a
            // configuration listing a logger repeatedly must not be refused for
            // a cardinality it does not have.
            let staged = names(&["api", "api", "api", "api", "api", "api"]);
            ensure_registry_capacity(&staged).expect("repeats count once");
        });
    }

    #[test]
    #[serial]
    fn already_registered_names_are_not_counted_again() {
        Python::attach(|py| {
            reset_manager();
            let _ = get_logger(py, "already.here").expect("logger registered");

            ensure_registry_capacity(&names(&["already.here"]))
                .expect("re-staging an existing logger adds nothing");
        });
    }

    #[test]
    #[serial]
    fn root_is_exempt_from_the_cap() {
        Python::attach(|py| {
            reset_manager();

            // Root is both absent-and-staged and then present, and neither
            // arrangement may count towards the cap.
            ensure_registry_capacity(&names(&["root"]))
                .expect("a staged root is exempt from the cap");
            let _ = get_logger(py, "root").expect("root is registrable");
            ensure_registry_capacity(&names(&["root"]))
                .expect("an existing root is exempt from the cap");
        });
    }

    #[test]
    #[serial]
    fn distinct_novel_names_count_once_each() {
        Python::attach(|py| {
            reset_manager();
            let _ = get_logger(py, "root").expect("root is registrable");

            let staged = names(&["a", "b.c", "b", "a"]);
            let mgr = MANAGER.read();
            let additions = new_logger_names(&mgr, staged.iter().map(String::as_str));
            assert_eq!(
                additions.len(),
                3,
                "three distinct novel names should be counted, got {additions:?}",
            );
        });
    }
}

#[cfg(feature = "log-compat")]
mod log_compat {
    //! Tests for the log-compat bridge's flush sweep.

    use std::any::Any;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use pyo3::Python;
    use serial_test::serial;

    use super::super::registry::{MANAGER, flush_all_handlers, get_logger, reset_manager};
    use crate::handler::{FemtoHandlerTrait, HandlerError};
    use crate::log_record::FemtoLogRecord;

    #[derive(Clone)]
    struct FlushCountingHandler {
        flushes: Arc<AtomicUsize>,
    }

    impl FemtoHandlerTrait for FlushCountingHandler {
        fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
            Ok(())
        }

        fn flush(&self) -> bool {
            self.flushes.fetch_add(1, Ordering::SeqCst);
            true
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    // `#[serial]` wraps the test bodies below, so the expect lint cannot
    // recognise them as tests; errors are propagated instead.
    #[test]
    #[serial]
    fn flush_all_handlers_flushes_loggers_with_handlers() -> pyo3::PyResult<()> {
        Python::attach(|py| -> pyo3::PyResult<()> {
            reset_manager();

            let flushes = Arc::new(AtomicUsize::new(0));
            let handler = Arc::new(FlushCountingHandler {
                flushes: flushes.clone(),
            }) as Arc<dyn FemtoHandlerTrait>;

            let logger_a = get_logger(py, "bridge.flush.a")?;
            let logger_b = get_logger(py, "bridge.flush.b")?;
            logger_a.borrow(py).add_handler(handler.clone());
            logger_b.borrow(py).add_handler(handler.clone());

            flush_all_handlers(py);

            assert_eq!(
                flushes.load(Ordering::SeqCst),
                2,
                "flush should be invoked once per logger with handlers",
            );
            Ok(())
        })
    }

    #[test]
    #[serial]
    fn flush_all_handlers_invokes_flush_once_per_registered_logger() -> pyo3::PyResult<()> {
        Python::attach(|py| -> pyo3::PyResult<()> {
            reset_manager();

            // Populate the manager with multiple loggers (including parents).
            let _ = get_logger(py, "bridge.flush.a")?;
            let _ = get_logger(py, "bridge.flush.b")?;

            let flushes = Arc::new(AtomicUsize::new(0));
            let handler = Arc::new(FlushCountingHandler {
                flushes: flushes.clone(),
            }) as Arc<dyn FemtoHandlerTrait>;

            let loggers = {
                let mgr = MANAGER.read();
                mgr.loggers
                    .values()
                    .map(|logger| logger.clone_ref(py))
                    .collect::<Vec<_>>()
            };

            for logger in &loggers {
                logger.borrow(py).add_handler(handler.clone());
            }

            flush_all_handlers(py);

            assert_eq!(
                flushes.load(Ordering::SeqCst),
                loggers.len(),
                "flush should be invoked once per registered logger",
            );
            Ok(())
        })
    }
}
