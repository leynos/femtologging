//! Capacity tests for the commit-time check.
//!
//! Split from `manager_tests.rs` to stay inside the repository's 400-line
//! module limit; the split is by concern, not by size alone: these are the
//! tests for planning a *set* of names, where `manager_tests.rs` covers the
//! registry itself.
//!
//! These exercise the counting rules — dedup, already-registered names and
//! the exempt root — rather than the boundary, which the pure arithmetic
//! tests in [`super`] cover.

use std::{
    any::Any,
    collections::{BTreeMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use pyo3::Python;
use serial_test::serial;

use crate::{
    handler::{FemtoHandlerTrait, HandlerError},
    log_record::FemtoLogRecord,
    logger::HandlerAttachment,
};

use super::registry::{
    MANAGER, ensure_registry_capacity, get_logger, new_logger_names, register_missing_ancestors,
    stage_loggers,
};
use super::{
    MAX_LOGGERS, PreparedConfiguration, PreparedLogger, commit_configuration, reset_manager,
};

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
        ensure_registry_capacity(&names(&["root"])).expect("a staged root is exempt from the cap");
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
            "a, b and b.c are three distinct entries, and the repeats of a \
             count once; the ancestor of b.c adds b, which the list already \
             names. Got {additions:?}",
        );
    });
}

/// A staged dotted name brings its ancestors with it, outermost first.
#[test]
#[serial]
fn an_ancestor_is_counted_before_its_descendant() {
    Python::attach(|py| {
        reset_manager();
        let _ = get_logger(py, "root").expect("root is registrable");

        let staged = names(&["api.v1.handlers"]);
        let mgr = MANAGER.read();
        let additions = new_logger_names(&mgr, staged.iter().map(String::as_str));
        assert_eq!(
            additions,
            vec!["api", "api.v1", "api.v1.handlers"],
            "the chain must be planned outermost first so each parent exists \
             before the child that names it",
        );
    });
}

#[cfg(feature = "log-compat")]
mod full_registry_bridge {
    //! Bridge filtering at the registry's capacity boundary.
    //!
    //! A target the registry cannot admit is filtered by root's live threshold.
    //! This is the branch [`get_logger`](super::super::registry::get_logger)
    //! refuses on, and the Rust `log` bridge has to predict it without Python:
    //! while a target still fits, dispatch would create it at the default
    //! level; once the registry is full, dispatch falls back to root instead.
    //! Filtering by the default in that second case would drop records root
    //! accepts whenever root is configured wider than `INFO`, which is exactly
    //! what issue #437 is about.

    use log::{Log, Metadata};
    use pyo3::Python;
    use serial_test::serial;

    use super::super::registry::{MANAGER, get_logger, reset_manager};
    use super::super::{MAX_LOGGERS, logger_registration_fits};
    use crate::FemtoLogAdapter;
    use crate::level::FemtoLevel;

    /// Build a `DEBUG` record aimed at `target`.
    fn debug_record(target: &str) -> Metadata<'_> {
        Metadata::builder()
            .level(log::Level::Debug)
            .target(target)
            .build()
    }

    #[test]
    #[serial]
    fn unregistrable_targets_filter_by_root_and_registrable_ones_keep_the_default() {
        Python::attach(|py| {
            reset_manager();
            log::set_max_level(log::LevelFilter::Trace);
            let adapter = FemtoLogAdapter;
            let target = "brand.new.target";

            let root = get_logger(py, "root").expect("root is always registrable");
            root.borrow(py).set_level(FemtoLevel::Debug);

            // Nothing is registered for the target, but it still fits, so
            // dispatch would create it at the default level rather than send
            // it to root's wider threshold.
            assert!(logger_registration_fits(target));
            assert!(
                !Log::enabled(&adapter, &debug_record(target)),
                "a target that would be created at the default level must not \
                 be filtered by root's wider threshold",
            );

            // Reach the bound with aliases of the one live logger. The policy
            // asks only whether a key is present and how many keys there are,
            // so this exercises the real capacity state without building ten
            // thousand loggers, each of which owns a dispatch worker.
            {
                let mut mgr = MANAGER.write();
                for index in 0..MAX_LOGGERS {
                    mgr.loggers
                        .insert(format!("saturated-{index}"), root.clone_ref(py));
                }
            }

            assert!(
                !logger_registration_fits(target),
                "a three-name chain cannot fit once the cap is reached",
            );
            assert!(
                Log::enabled(&adapter, &debug_record(target)),
                "an unregistrable target is routed to root, so root's live \
                 threshold must decide whether its records are enabled",
            );
            assert!(
                logger_registration_fits("root"),
                "the fallback destination is exempt from the cap and must \
                 always remain admissible",
            );
            assert!(
                !Log::enabled(
                    &adapter,
                    &Metadata::builder()
                        .level(log::Level::Trace)
                        .target(target)
                        .build(),
                ),
                "root's threshold still rejects levels it does not accept",
            );
        });
        reset_manager();
    }
}

/// A configuration over the cap is rejected before its logger objects are staged.
#[test]
#[serial]
fn staging_refuses_more_distinct_names_than_the_registry_can_hold() {
    Python::attach(|py| {
        reset_manager();
        let requested: Vec<String> = (0..=MAX_LOGGERS)
            .map(|index| format!("staged-{index}"))
            .collect();

        let result = stage_loggers(py, requested.iter().map(String::as_str));

        assert!(
            result.is_err(),
            "staging must reject a set whose distinct names exceed the cap"
        );
        assert!(
            MANAGER.read().loggers.is_empty(),
            "a refused staged set must not change the registry"
        );
    });
}

/// The commit registers missing ancestors and leaves staged names to its caller.
#[test]
#[serial]
fn a_commit_registers_missing_ancestors_but_not_staged_names() {
    Python::attach(|py| {
        reset_manager();
        {
            let mut mgr = MANAGER.write();
            register_missing_ancestors(py, &mut mgr, std::iter::once("a.b.c"), |name| {
                name == "a.b.c"
            })
            .expect("the chain fits the empty registry");
        }

        assert_eq!(
            super::registered_logger_names_for_test(),
            vec!["a", "a.b"],
            "the commit must create the unconfigured ancestors and leave the \
             configured name for the caller's staged logger object",
        );
    });
}

/// A commit over the cap is refused before it registers anything.
#[test]
#[serial]
fn a_commit_over_the_cap_registers_nothing() {
    Python::attach(|py| {
        reset_manager();
        let requested: Vec<String> = (0..=MAX_LOGGERS)
            .map(|index| format!("committed-{index}"))
            .collect();

        let mut mgr = MANAGER.write();
        let result =
            register_missing_ancestors(py, &mut mgr, requested.iter().map(String::as_str), |_| {
                true
            });

        assert!(
            result.is_err(),
            "a commit whose planned chains exceed the cap must be refused"
        );
        assert!(
            mgr.loggers.is_empty(),
            "a refused commit must not leave a partly registered chain"
        );
    });
}

/// Handler that records whether the manager lock was free when it was dropped.
struct LockProbeHandler {
    /// Set to true when `Drop` could take the manager write lock.
    lock_was_free: Arc<AtomicBool>,
}

impl Drop for LockProbeHandler {
    fn drop(&mut self) {
        // `try_write` fails while the committing thread still holds the lock,
        // which is exactly the state the refusal path must avoid: a handler
        // that flushes or joins a worker would otherwise wait on a lock its
        // own thread is holding.
        let acquired = MANAGER.try_write();
        self.lock_was_free
            .store(acquired.is_some(), Ordering::SeqCst);
    }
}

impl FemtoHandlerTrait for LockProbeHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A refused commit releases the manager lock before dropping staged handlers.
#[test]
#[serial]
fn a_refused_commit_drops_staged_handlers_after_unlocking() {
    Python::attach(|py| {
        reset_manager();
        let staged = stage_loggers(py, std::iter::once("probe"))
            .expect("one staged logger fits an empty registry");
        let probe_logger = staged
            .get("probe")
            .expect("the staged map holds the requested name")
            .clone_ref(py);

        let lock_was_free = Arc::new(AtomicBool::new(false));
        let probe_plan = PreparedLogger {
            name: "refused-0".to_string(),
            logger: probe_logger.clone_ref(py),
            handlers: vec![HandlerAttachment::new(Arc::new(LockProbeHandler {
                lock_was_free: lock_was_free.clone(),
            }))],
            filters: Vec::new(),
            handler_ids: Vec::new(),
            filter_ids: Vec::new(),
            level: None,
            propagate: None,
        };
        // One more distinct name than the registry can hold, so the commit is
        // refused after staging but before any plan is applied.
        let remaining = (1..=MAX_LOGGERS).map(|index| PreparedLogger {
            name: format!("refused-{index}"),
            logger: probe_logger.clone_ref(py),
            handlers: Vec::new(),
            filters: Vec::new(),
            handler_ids: Vec::new(),
            filter_ids: Vec::new(),
            level: None,
            propagate: None,
        });
        let loggers: Vec<PreparedLogger> = std::iter::once(probe_plan).chain(remaining).collect();

        let result = commit_configuration(
            py,
            PreparedConfiguration {
                loggers,
                disable_existing: false,
                keep_names: HashSet::new(),
                handler_registry: BTreeMap::new(),
                handler_filter_ids: BTreeMap::new(),
                filter_registry: BTreeMap::new(),
            },
        );

        assert!(result.is_err(), "a commit over the cap must be refused");
        assert!(
            lock_was_free.load(Ordering::SeqCst),
            "staged handlers must be dropped after the manager lock is released",
        );
        assert!(
            MANAGER.read().loggers.is_empty(),
            "a refused commit must not register anything",
        );
    });
}
