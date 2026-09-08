//! Tests for the logger manager registry.

#[cfg(feature = "python")]
mod lookup {
    //! Tests for read-only logger lookup.

    use pyo3::Python;
    use serial_test::serial;

    use super::super::{MANAGER, get_logger, lookup_existing_logger, reset_manager};

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

#[cfg(feature = "log-compat")]
mod log_compat {
    //! Tests for the log-compat bridge integration.

    use std::any::Any;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use pyo3::Python;
    use serial_test::serial;

    use super::super::{MANAGER, flush_all_handlers, get_logger, reset_manager};
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
