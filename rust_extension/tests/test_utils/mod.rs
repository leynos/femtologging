pub mod fixtures;
pub mod shared_buffer;

pub mod std {
    pub use super::fixtures::handler_tuple;
    pub use super::shared_buffer::std::{
        read_output, Arc as StdArc, Barrier as StdBarrier, Mutex as StdMutex, SharedBuf,
    };
}

pub mod loom {
    pub use super::shared_buffer::loom::{
        read_output, Arc as LoomArc, Barrier as LoomBarrier, Mutex as LoomMutex, SharedBuf,
    };
}
