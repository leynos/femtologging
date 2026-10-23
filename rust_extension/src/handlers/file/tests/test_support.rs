//! Test-only rotation strategies used by file-handler unit tests.

use std::io::{self, Seek, Write};

/// Strategy that returns an error to exercise rotation-failure handling.
pub(crate) struct FailingRotation;

impl<W: Write + Seek> super::super::RotationStrategy<W> for FailingRotation {
    fn before_write(&mut self, _writer: &mut W, _formatted: &str) -> io::Result<bool> {
        Err(io::Error::other("failing rotation for test"))
    }
}
