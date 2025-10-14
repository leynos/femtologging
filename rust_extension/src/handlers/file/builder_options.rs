//! Builder options for constructing [`FemtoFileHandler`].
//!
//! The options configure rotation strategy and synchronisation primitives used
//! when spawning the worker thread in tests and custom setups.

use std::io::{Seek, Write};
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::Barrier;

use super::{NoRotation, RotationStrategy};

/// Options used when spawning the file handler worker thread.
///
/// These options primarily exist for tests to inject alternative rotation
/// strategies and coordinate worker start-up using a [`Barrier`].
pub(crate) struct BuilderOptions<W, R = NoRotation>
where
    W: Write + Seek,
    R: RotationStrategy<W>,
{
    pub(crate) rotation: R,
    pub(crate) start_barrier: Option<Arc<Barrier>>,
    pub(crate) _phantom: PhantomData<W>,
}

impl<W> Default for BuilderOptions<W>
where
    W: Write + Seek,
{
    fn default() -> Self {
        Self {
            rotation: NoRotation,
            start_barrier: None,
            _phantom: PhantomData,
        }
    }
}

impl<W, R> BuilderOptions<W, R>
where
    W: Write + Seek,
    R: RotationStrategy<W>,
{
    pub(crate) fn new(rotation: R, start_barrier: Option<Arc<Barrier>>) -> Self {
        Self {
            rotation,
            start_barrier,
            _phantom: PhantomData,
        }
    }
}
