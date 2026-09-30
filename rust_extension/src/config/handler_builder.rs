//! Concrete handler builder variants and their shared build operations.

use std::sync::Arc;

#[cfg(feature = "python")]
use std::collections::BTreeMap;

use crate::{
    handler::FemtoHandlerTrait,
    handlers::{
        FileHandlerBuilder, HTTPHandlerBuilder, HandlerBuildError, HandlerBuilderTrait,
        RotatingFileHandlerBuilder, SocketHandlerBuilder, StreamHandlerBuilder,
        TimedRotatingFileHandlerBuilder,
    },
};

#[cfg(feature = "python")]
use crate::formatter::SharedFormatter;

/// Concrete handler builder variants accepted by [`crate::ConfigBuilder::with_handler`].
///
/// Each variant carries the corresponding public handler builder. In
/// particular, [`HandlerBuilder::Http`] carries an [`HTTPHandlerBuilder`]; its
/// configured filter IDs are resolved when the containing configuration is
/// built.
///
/// # Examples
///
/// ```rust
/// use _femtologging_rs::{HandlerBuilder, HTTPHandlerBuilder};
///
/// let builder: HandlerBuilder = HTTPHandlerBuilder::new()
///     .with_url("https://example.invalid/logs")
///     .into();
/// assert!(matches!(builder, HandlerBuilder::Http(_)));
/// ```
#[derive(Clone, Debug)]
pub enum HandlerBuilder {
    /// Build a [`FemtoStreamHandler`].
    Stream(StreamHandlerBuilder),
    /// Build a [`FemtoFileHandler`].
    File(FileHandlerBuilder),
    /// Build a [`FemtoRotatingFileHandler`].
    Rotating(RotatingFileHandlerBuilder),
    /// Build a [`FemtoTimedRotatingFileHandler`].
    TimedRotating(TimedRotatingFileHandlerBuilder),
    /// Build a [`FemtoSocketHandler`].
    Socket(SocketHandlerBuilder),
    /// Build a [`FemtoHTTPHandler`] from an [`HTTPHandlerBuilder`].
    ///
    /// Filter IDs set on the builder are resolved by `ConfigBuilder` before
    /// the HTTP handler is attached to a logger.
    Http(HTTPHandlerBuilder),
}

impl HandlerBuilder {
    #[cfg_attr(
        not(feature = "python"),
        expect(dead_code, reason = "unused without python feature")
    )]
    pub(crate) fn build(&self) -> Result<Arc<dyn FemtoHandlerTrait>, HandlerBuildError> {
        let mut builder = self.clone();
        builder.clear_filter_ids();
        match &builder {
            Self::Stream(b) => <StreamHandlerBuilder as HandlerBuilderTrait>::build_inner(b)
                .map(|handler| Arc::new(handler) as Arc<dyn FemtoHandlerTrait>),
            Self::File(b) => <FileHandlerBuilder as HandlerBuilderTrait>::build_inner(b)
                .map(|handler| Arc::new(handler) as Arc<dyn FemtoHandlerTrait>),
            Self::Rotating(b) => {
                <RotatingFileHandlerBuilder as HandlerBuilderTrait>::build_inner(b)
                    .map(|handler| Arc::new(handler) as Arc<dyn FemtoHandlerTrait>)
            }
            Self::TimedRotating(b) => {
                <TimedRotatingFileHandlerBuilder as HandlerBuilderTrait>::build_inner(b)
                    .map(|handler| Arc::new(handler) as Arc<dyn FemtoHandlerTrait>)
            }
            Self::Socket(b) => <SocketHandlerBuilder as HandlerBuilderTrait>::build_inner(b)
                .map(|handler| Arc::new(handler) as Arc<dyn FemtoHandlerTrait>),
            Self::Http(b) => <HTTPHandlerBuilder as HandlerBuilderTrait>::build_inner(b)
                .map(|handler| Arc::new(handler) as Arc<dyn FemtoHandlerTrait>),
        }
    }

    fn clear_filter_ids(&mut self) {
        match self {
            Self::Stream(builder) => *builder = builder.clone().with_filters(Vec::<String>::new()),
            Self::File(builder) => *builder = builder.clone().with_filters(Vec::<String>::new()),
            Self::Rotating(builder) => {
                *builder = builder.clone().with_filters(Vec::<String>::new());
            }
            Self::TimedRotating(builder) => {
                *builder = builder.clone().with_filters(Vec::<String>::new());
            }
            Self::Socket(builder) => *builder = builder.clone().with_filters(Vec::<String>::new()),
            Self::Http(builder) => *builder = builder.clone().with_filters(Vec::<String>::new()),
        }
    }

    /// Build this handler after resolving its configured formatter identifier.
    #[cfg(feature = "python")]
    pub(crate) fn build_with_formatters(
        &self,
        formatters: &BTreeMap<String, SharedFormatter>,
    ) -> Result<Arc<dyn FemtoHandlerTrait>, HandlerBuildError> {
        let mut builder = self.clone();
        match &mut builder {
            Self::Stream(builder) => builder.resolve_formatter(formatters)?,
            Self::File(builder) => builder.resolve_formatter(formatters)?,
            Self::Rotating(builder) => builder.resolve_formatter(formatters)?,
            Self::TimedRotating(builder) => builder.resolve_formatter(formatters)?,
            Self::Socket(_) | Self::Http(_) => {}
        }
        builder.build()
    }

    /// Return filter identifiers configured on this handler.
    #[cfg(feature = "python")]
    pub(crate) fn filter_ids(&self) -> &[String] {
        match self {
            Self::Stream(builder) => builder.filter_ids(),
            Self::File(builder) => builder.filter_ids(),
            Self::Rotating(builder) => builder.filter_ids(),
            Self::TimedRotating(builder) => builder.filter_ids(),
            Self::Socket(builder) => builder.filter_ids(),
            Self::Http(builder) => builder.filter_ids(),
        }
    }
}

impl From<StreamHandlerBuilder> for HandlerBuilder {
    fn from(value: StreamHandlerBuilder) -> Self {
        Self::Stream(value)
    }
}

impl From<FileHandlerBuilder> for HandlerBuilder {
    fn from(value: FileHandlerBuilder) -> Self {
        Self::File(value)
    }
}

impl From<RotatingFileHandlerBuilder> for HandlerBuilder {
    fn from(value: RotatingFileHandlerBuilder) -> Self {
        Self::Rotating(value)
    }
}

impl From<TimedRotatingFileHandlerBuilder> for HandlerBuilder {
    fn from(value: TimedRotatingFileHandlerBuilder) -> Self {
        Self::TimedRotating(value)
    }
}

impl From<SocketHandlerBuilder> for HandlerBuilder {
    fn from(value: SocketHandlerBuilder) -> Self {
        Self::Socket(value)
    }
}

impl From<HTTPHandlerBuilder> for HandlerBuilder {
    fn from(value: HTTPHandlerBuilder) -> Self {
        Self::Http(value)
    }
}
