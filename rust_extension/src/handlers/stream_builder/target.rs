//! Stream-target selection for the stream handler builder.

/// Standard stream destinations supported by [`super::StreamHandlerBuilder`].
#[derive(Clone, Copy, Debug)]
pub(super) enum StreamTarget {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

impl StreamTarget {
    /// Return the Python configuration identifier for this destination.
    #[cfg(feature = "python")]
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}
