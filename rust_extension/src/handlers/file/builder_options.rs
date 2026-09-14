//! Builder-only options for constructing file handlers.
//!
//! These options are only needed while wiring a worker during handler
//! construction. Keeping them in a dedicated module shortens the public
//! handler module without changing the exported API.

use std::{
    io::{Seek, Write},
    marker::PhantomData,
    sync::{Arc, Barrier},
};

use super::{NoRotation, RotationStrategy};

/// Builder-only state needed to start a file-handler worker.
///
/// The public builders construct this value just before spawning the worker so
/// rotation hooks and test-only synchronisation can stay out of the runtime
/// handler state.
pub(crate) struct BuilderOptions<W, R = NoRotation>
where
    W: Write + Seek,
    R: RotationStrategy<W>,
{
    /// Owns the rotation strategy transferred to the file worker, which performs all rollover I/O on its own thread.
    pub(crate) rotation: R,
    /// Holds an optional test barrier released by the worker after startup; production construction leaves it unset.
    pub(crate) start_barrier: Option<Arc<Barrier>>,
    /// Retains the writer type at compile time without storing a second writer value.
    _phantom: PhantomData<W>,
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
    /// Bundle rotation behaviour and an optional start barrier for worker
    /// construction.
    pub(crate) fn new(rotation: R, start_barrier: Option<Arc<Barrier>>) -> Self {
        Self {
            rotation,
            start_barrier,
            _phantom: PhantomData,
        }
    }
}
