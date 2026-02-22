//! Rust logging macros that capture source location.
//!
//! These macros provide ergonomic logging for Rust callers within the
//! extension crate or downstream crates linking against `femtologging_rs`.
//! Each macro captures `file!()`, `line!()`, and `module_path!()` at the
//! call site and embeds them in the log record's [`RecordMetadata`].
//!
//! The macros are prefixed with `femtolog_` to avoid collision with the
//! `log` crate's identically named macros that are already used internally.
//!
//! # Examples
//!
//! ```rust,ignore
//! use femtologging_rs::FemtoLogger;
//!
//! let logger = FemtoLogger::new("example".into());
//! femtolog_info!(logger, "server started on port 8080");
//! femtolog_error!(logger, "connection failed");
//! ```
//!
//! [`RecordMetadata`]: crate::log_record::RecordMetadata

/// Log a message at `DEBUG` level, capturing the call site's source location.
///
/// # Examples
///
/// ```rust,ignore
/// let logger = FemtoLogger::new("app".into());
/// femtolog_debug!(logger, "entering request handler");
/// ```
#[macro_export]
macro_rules! femtolog_debug {
    ($logger:expr, $message:expr) => {
        $crate::__femtolog_impl!($logger, $crate::FemtoLevel::Debug, $message)
    };
}

/// Log a message at `INFO` level, capturing the call site's source location.
///
/// # Examples
///
/// ```rust,ignore
/// let logger = FemtoLogger::new("app".into());
/// femtolog_info!(logger, "server started");
/// ```
#[macro_export]
macro_rules! femtolog_info {
    ($logger:expr, $message:expr) => {
        $crate::__femtolog_impl!($logger, $crate::FemtoLevel::Info, $message)
    };
}

/// Log a message at `WARN` level, capturing the call site's source location.
///
/// # Examples
///
/// ```rust,ignore
/// let logger = FemtoLogger::new("app".into());
/// femtolog_warn!(logger, "disk space running low");
/// ```
#[macro_export]
macro_rules! femtolog_warn {
    ($logger:expr, $message:expr) => {
        $crate::__femtolog_impl!($logger, $crate::FemtoLevel::Warn, $message)
    };
}

/// Log a message at `ERROR` level, capturing the call site's source location.
///
/// # Examples
///
/// ```rust,ignore
/// let logger = FemtoLogger::new("app".into());
/// femtolog_error!(logger, "failed to open database");
/// ```
#[macro_export]
macro_rules! femtolog_error {
    ($logger:expr, $message:expr) => {
        $crate::__femtolog_impl!($logger, $crate::FemtoLevel::Error, $message)
    };
}

/// Internal implementation macro — not part of the public API.
///
/// Captures `file!()`, `line!()`, and `module_path!()` at the expansion site
/// (which is the caller's site because the public macros delegate here via
/// `$crate`). Constructs a [`RecordMetadata`] and calls
/// [`FemtoLogger::log_with_metadata`].
#[doc(hidden)]
#[macro_export]
macro_rules! __femtolog_impl {
    ($logger:expr, $level:expr, $message:expr) => {{
        let metadata = $crate::RecordMetadata {
            module_path: ::std::string::String::from(::std::module_path!()),
            filename: ::std::string::String::from(::std::file!()),
            line_number: ::std::line!(),
            ..::std::default::Default::default()
        };
        $logger.log_with_metadata($level, $message, metadata)
    }};
}

#[cfg(test)]
mod tests {
    //! Unit tests for the femtologging macros.

    use crate::handler::{FemtoHandlerTrait, HandlerError};
    use crate::log_record::FemtoLogRecord;
    use crate::logger::FemtoLogger;
    use parking_lot::Mutex;
    use rstest::rstest;
    use std::any::Any;
    use std::sync::Arc;

    #[derive(Clone, Default)]
    struct CollectingHandler {
        records: Arc<Mutex<Vec<FemtoLogRecord>>>,
    }

    impl CollectingHandler {
        fn collected(&self) -> Vec<FemtoLogRecord> {
            self.records.lock().clone()
        }
    }

    impl FemtoHandlerTrait for CollectingHandler {
        fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
            self.records.lock().push(record);
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[rstest]
    fn debug_macro_dispatches_at_debug_level() {
        let logger = FemtoLogger::new("macro.test".into());
        logger.set_level(crate::FemtoLevel::Debug);
        let handler = Arc::new(CollectingHandler::default());
        logger.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let result = femtolog_debug!(logger, "debug message");
        assert!(result.is_some());
        assert!(logger.flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level_str(), "DEBUG");
        assert_eq!(records[0].message(), "debug message");
    }

    #[rstest]
    fn info_macro_dispatches_at_info_level() {
        let logger = FemtoLogger::new("macro.test".into());
        let handler = Arc::new(CollectingHandler::default());
        logger.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let result = femtolog_info!(logger, "info message");
        assert!(result.is_some());
        assert!(logger.flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level_str(), "INFO");
    }

    #[rstest]
    fn warn_macro_dispatches_at_warn_level() {
        let logger = FemtoLogger::new("macro.test".into());
        let handler = Arc::new(CollectingHandler::default());
        logger.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let result = femtolog_warn!(logger, "warn message");
        assert!(result.is_some());
        assert!(logger.flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level_str(), "WARN");
    }

    #[rstest]
    fn error_macro_dispatches_at_error_level() {
        let logger = FemtoLogger::new("macro.test".into());
        let handler = Arc::new(CollectingHandler::default());
        logger.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let result = femtolog_error!(logger, "error message");
        assert!(result.is_some());
        assert!(logger.flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level_str(), "ERROR");
    }

    #[rstest]
    fn macro_captures_source_location() {
        let logger = FemtoLogger::new("macro.test".into());
        let handler = Arc::new(CollectingHandler::default());
        logger.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let _result = femtolog_info!(logger, "located");
        assert!(logger.flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        let meta = records[0].metadata();
        assert!(
            meta.filename.contains("logging_macros.rs"),
            "filename should contain this file: got {:?}",
            meta.filename
        );
        assert!(meta.line_number > 0, "line number should be positive");
        assert!(
            meta.module_path.contains("logging_macros"),
            "module_path should contain this module: got {:?}",
            meta.module_path
        );
    }

    #[rstest]
    fn below_threshold_returns_none() {
        let logger = FemtoLogger::new("macro.test".into());
        // Default level is INFO, so DEBUG should be filtered out
        let result = femtolog_debug!(logger, "should be filtered");
        assert!(result.is_none());
    }
}
