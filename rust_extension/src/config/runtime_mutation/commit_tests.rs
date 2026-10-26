//! Regression tests for runtime mutation commit atomicity.

use std::collections::BTreeMap;

use pyo3::Python;
use serial_test::serial;

use super::*;
use crate::{FemtoLevel, manager};

#[test]
#[serial]
fn later_logger_resolution_failure_keeps_earlier_logger_unchanged() {
    Python::attach(|_| manager::reset_manager());

    Python::attach(|py| {
        let first = manager::get_logger(py, "first").expect("first logger should be created");
        let second = manager::get_logger(py, "second").expect("second logger should be created");
        first.borrow(py).set_level(FemtoLevel::Info);

        let _previous_runtime = manager::replace_runtime_state(
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::from([("sentinel".to_string(), LoggerAttachmentState::default())]),
        );
        let before = manager::snapshot_runtime_state();
        let mut logger_states = before.logger_states.clone();
        logger_states.insert("first".to_string(), LoggerAttachmentState::default());
        logger_states.insert(
            "second".to_string(),
            LoggerAttachmentState::new(vec!["missing".to_string()], Vec::new()),
        );

        let commit = RuntimeCommit {
            logger_states,
            handler_registry: before.handler_registry.clone(),
            handler_filter_ids: before.handler_filter_ids.clone(),
            resolved_handler_filters: BTreeMap::new(),
            filter_registry: before.filter_registry.clone(),
            impacted_loggers: vec![
                ("first".to_string(), first.clone_ref(py)),
                ("second".to_string(), second),
            ],
            scalar_mutations: BTreeMap::from([(
                "first".to_string(),
                LoggerScalarMutation {
                    level: Some(FemtoLevel::Debug),
                    propagate: None,
                },
            )]),
        };

        let error = apply_commit(py, &commit).expect_err("missing attachment should be rejected");
        assert!(matches!(
            error,
            ConfigError::InvalidMutation(message)
                if message.contains("missing handler attachment id")
        ));
        assert_eq!(
            first.borrow(py).level(),
            "INFO",
            "a later resolution error must not apply earlier logger changes",
        );

        let after = manager::snapshot_runtime_state();
        assert_eq!(after.logger_states, before.logger_states);
        assert_eq!(after.handler_filter_ids, before.handler_filter_ids);
        assert_eq!(
            after.handler_registry.keys().collect::<Vec<_>>(),
            before.handler_registry.keys().collect::<Vec<_>>(),
        );
        assert_eq!(
            after.filter_registry.keys().collect::<Vec<_>>(),
            before.filter_registry.keys().collect::<Vec<_>>(),
        );
    });

    Python::attach(|_| manager::reset_manager());
}
