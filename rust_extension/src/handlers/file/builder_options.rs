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

pub(crate) struct BuilderOptions<W, R = NoRotation>
where
    W: Write + Seek,
    R: RotationStrategy<W>,
{
    pub(crate) rotation: R,
    pub(crate) start_barrier: Option<Arc<Barrier>>,
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
    pub(crate) fn new(rotation: R, start_barrier: Option<Arc<Barrier>>) -> Self {
        Self {
            rotation,
            start_barrier,
            _phantom: PhantomData,
        }
    }
}
