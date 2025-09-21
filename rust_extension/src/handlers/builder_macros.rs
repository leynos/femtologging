//! Macros for generating shared builder methods.
//!
//! The builder structs expose identical fluent APIs to Rust and Python callers.
//! These macros centralise the shared method definitions so the two bindings
//! remain in sync and avoid repetitive boilerplate.

macro_rules! builder_method_rust {
    (
        $doc:expr,
        $rust_fn:ident,
        $py_fn:ident,
        $py_name:literal,
        ( $( $rarg:ident : $rty:ty ),* ),
        ( $( $parg:ident : $pty:ty ),* ),
        $builder:ident,
        $body:block
    ) => {
        #[doc = $doc]
        pub fn $rust_fn(mut self, $( $rarg : $rty ),* ) -> Self {
            let $builder = &mut self;
            $body
            self
        }
    };
}
pub(crate) use builder_method_rust;

macro_rules! file_like_builder_methods {
    ($callback:ident) => {
        $callback!(
            "Set the bounded channel capacity.",
            with_capacity,
            py_with_capacity,
            "with_capacity",
            (capacity: usize),
            (capacity: usize),
            builder,
            {
                builder.state.set_capacity(capacity);
            }
        );
        $callback!(
            "Set the periodic flush interval measured in records. Must be greater than zero.",
            with_flush_record_interval,
            py_with_flush_record_interval,
            "with_flush_record_interval",
            (interval: usize),
            (interval: usize),
            builder,
            {
                builder.state.set_flush_record_interval(interval);
            }
        );
        $callback!(
            "Set the formatter identifier.",
            with_formatter,
            py_with_formatter,
            "with_formatter",
            (formatter_id: impl Into<FormatterId>),
            (formatter_id: String),
            builder,
            {
                builder.state.set_formatter(formatter_id);
            }
        );
    };
}
pub(crate) use file_like_builder_methods;

macro_rules! stream_builder_methods {
    ($callback:ident) => {
        $callback!(
            "Set the bounded channel capacity.",
            with_capacity,
            py_with_capacity,
            "with_capacity",
            (capacity: usize),
            (capacity: usize),
            builder,
            {
                builder.common.capacity = NonZeroUsize::new(capacity);
                builder.common.capacity_set = true;
            }
        );
        $callback!(
            "Set the flush timeout in milliseconds. Must be greater than zero.",
            with_flush_timeout_ms,
            py_with_flush_timeout_ms,
            "with_flush_timeout_ms",
            (timeout_ms: u64),
            (timeout_ms: u64),
            builder,
            {
                builder.common.flush_timeout_ms = Some(timeout_ms);
            }
        );
        $callback!(
            "Set the formatter identifier.",
            with_formatter,
            py_with_formatter,
            "with_formatter",
            (formatter_id: impl Into<FormatterId>),
            (formatter_id: String),
            builder,
            {
                builder.common.formatter_id = Some(formatter_id.into());
            }
        );
    };
}
pub(crate) use stream_builder_methods;

macro_rules! rotating_limit_methods {
    ($callback:ident) => {
        $callback!(
            "Set the maximum number of bytes before rotation occurs.",
            with_max_bytes,
            py_with_max_bytes,
            "with_max_bytes",
            (max_bytes: u64),
            (max_bytes: u64),
            builder,
            {
                builder.max_bytes = NonZeroU64::new(max_bytes);
                builder.max_bytes_set = true;
            }
        );
        $callback!(
            "Set how many backup files to retain during rotation.",
            with_backup_count,
            py_with_backup_count,
            "with_backup_count",
            (backup_count: usize),
            (backup_count: usize),
            builder,
            {
                builder.backup_count = NonZeroUsize::new(backup_count);
                builder.backup_count_set = true;
            }
        );
    };
}
pub(crate) use rotating_limit_methods;
