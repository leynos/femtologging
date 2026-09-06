//! Shared test utilities for the integration test crates.

pub mod fixtures;
pub mod handle_expect;
pub mod shared_buffer;

pub mod std {
    //! Re-exports selecting the std-backed shared buffer.

    pub use super::shared_buffer::std::SharedBuf;
}
