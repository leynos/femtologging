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
//! femtolog_info!(logger, "server started on port {}", 8080);
//! femtolog_error!(logger, "connection failed");
//! ```
//!
//! [`RecordMetadata`]: crate::log_record::RecordMetadata

/// Log a message at `DEBUG` level, capturing the call site's source location.
///
/// Accepts either a plain string or `format!`-style arguments.
///
/// # Examples
///
/// ```rust,ignore
/// let logger = FemtoLogger::new("app".into());
/// femtolog_debug!(logger, "entering request handler");
/// femtolog_debug!(logger, "request id = {}", request_id);
/// ```
#[macro_export]
macro_rules! femtolog_debug {
    ($logger:expr, $message:expr) => {
        $crate::__femtolog_impl!($logger, $crate::FemtoLevel::Debug, $message)
    };
    ($logger:expr, $fmt:expr, $($arg:tt)+) => {
        $crate::__femtolog_impl!($logger, $crate::FemtoLevel::Debug, $fmt, $($arg)+)
    };
}

/// Log a message at `INFO` level, capturing the call site's source location.
///
/// Accepts either a plain string or `format!`-style arguments.
///
/// # Examples
///
/// ```rust,ignore
/// let logger = FemtoLogger::new("app".into());
/// femtolog_info!(logger, "server started");
/// femtolog_info!(logger, "listening on port {}", port);
/// ```
#[macro_export]
macro_rules! femtolog_info {
    ($logger:expr, $message:expr) => {
        $crate::__femtolog_impl!($logger, $crate::FemtoLevel::Info, $message)
    };
    ($logger:expr, $fmt:expr, $($arg:tt)+) => {
        $crate::__femtolog_impl!($logger, $crate::FemtoLevel::Info, $fmt, $($arg)+)
    };
}

/// Log a message at `WARN` level, capturing the call site's source location.
///
/// Accepts either a plain string or `format!`-style arguments.
///
/// # Examples
///
/// ```rust,ignore
/// let logger = FemtoLogger::new("app".into());
/// femtolog_warn!(logger, "disk space running low");
/// femtolog_warn!(logger, "{}% disk used", usage);
/// ```
#[macro_export]
macro_rules! femtolog_warn {
    ($logger:expr, $message:expr) => {
        $crate::__femtolog_impl!($logger, $crate::FemtoLevel::Warn, $message)
    };
    ($logger:expr, $fmt:expr, $($arg:tt)+) => {
        $crate::__femtolog_impl!($logger, $crate::FemtoLevel::Warn, $fmt, $($arg)+)
    };
}

/// Log a message at `ERROR` level, capturing the call site's source location.
///
/// Accepts either a plain string or `format!`-style arguments.
///
/// # Examples
///
/// ```rust,ignore
/// let logger = FemtoLogger::new("app".into());
/// femtolog_error!(logger, "failed to open database");
/// femtolog_error!(logger, "connection to {} lost", host);
/// ```
#[macro_export]
macro_rules! femtolog_error {
    ($logger:expr, $message:expr) => {
        $crate::__femtolog_impl!($logger, $crate::FemtoLevel::Error, $message)
    };
    ($logger:expr, $fmt:expr, $($arg:tt)+) => {
        $crate::__femtolog_impl!($logger, $crate::FemtoLevel::Error, $fmt, $($arg)+)
    };
}

/// Internal implementation macro — not part of the public API.
///
/// Captures `file!()`, `line!()`, and `module_path!()` at the expansion site
/// (which is the caller's site because the public macros delegate here via
/// `$crate`). Constructs a [`RecordMetadata`] and calls
/// [`FemtoLogger::log_with_metadata`].
///
/// The first arm handles a pre-formed message string; the second accepts
/// `format!`-style arguments.
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
    ($logger:expr, $level:expr, $fmt:expr, $($arg:tt)+) => {{
        let metadata = $crate::RecordMetadata {
            module_path: ::std::string::String::from(::std::module_path!()),
            filename: ::std::string::String::from(::std::file!()),
            line_number: ::std::line!(),
            ..::std::default::Default::default()
        };
        $logger.log_with_metadata($level, &::std::format!($fmt, $($arg)+), metadata)
    }};
}

#[cfg(test)]
mod tests {
    //! Unit tests for the femtologging macros.

    use crate::handler::FemtoHandlerTrait;
    use crate::logger::FemtoLogger;
    use crate::test_utils::collecting_handler::CollectingHandler;
    use rstest::{fixture, rstest};
    use std::sync::Arc;

    /// Create a logger at DEBUG level with an attached `CollectingHandler`.
    ///
    /// The logger threshold is set to DEBUG so that all levels are accepted,
    /// allowing parameterised tests to exercise every macro variant.
    #[fixture]
    fn logger_with_handler() -> (FemtoLogger, Arc<CollectingHandler>) {
        let logger = FemtoLogger::new("macro.test".into());
        logger.set_level(crate::FemtoLevel::Debug);
        let handler = Arc::new(CollectingHandler::default());
        logger.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);
        (logger, handler)
    }

    /// Assert that `handler` collected exactly one record with the expected
    /// level string and message.
    fn assert_single_record(
        handler: &CollectingHandler,
        expected_level: &str,
        expected_message: &str,
    ) {
        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level_str(), expected_level);
        assert_eq!(records[0].message(), expected_message);
    }

    #[rstest]
    #[case::debug(crate::FemtoLevel::Debug, "DEBUG", "debug message")]
    #[case::info(crate::FemtoLevel::Info, "INFO", "info message")]
    #[case::warn(crate::FemtoLevel::Warn, "WARN", "warn message")]
    #[case::error(crate::FemtoLevel::Error, "ERROR", "error message")]
    fn macro_dispatches_at_level(
        logger_with_handler: (FemtoLogger, Arc<CollectingHandler>),
        #[case] level: crate::FemtoLevel,
        #[case] expected_level_str: &str,
        #[case] message: &str,
    ) {
        let (logger, handler) = logger_with_handler;
        let result = match level {
            crate::FemtoLevel::Debug => femtolog_debug!(logger, message),
            crate::FemtoLevel::Info => femtolog_info!(logger, message),
            crate::FemtoLevel::Warn => femtolog_warn!(logger, message),
            crate::FemtoLevel::Error => femtolog_error!(logger, message),
            _ => unreachable!("only debug/info/warn/error are tested"),
        };
        assert!(result.is_some());
        assert!(logger.flush_handlers());
        assert_single_record(&handler, expected_level_str, message);
    }

    #[rstest]
    fn macro_captures_source_location(logger_with_handler: (FemtoLogger, Arc<CollectingHandler>)) {
        let (logger, handler) = logger_with_handler;

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
    fn format_args_are_interpolated(logger_with_handler: (FemtoLogger, Arc<CollectingHandler>)) {
        let (logger, handler) = logger_with_handler;

        let port = 8080;
        let result = femtolog_info!(logger, "listening on port {}", port);
        assert!(result.is_some());
        assert!(logger.flush_handlers());

        assert_single_record(&handler, "INFO", "listening on port 8080");
    }

    #[rstest]
    fn below_threshold_returns_none() {
        // Use default INFO level so DEBUG is filtered out.
        let logger = FemtoLogger::new("macro.test".into());
        let handler = Arc::new(CollectingHandler::default());
        logger.add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let result = femtolog_debug!(logger, "should be filtered");
        assert!(result.is_none());
    }
}
