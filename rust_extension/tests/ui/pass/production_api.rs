//! Compile-pass coverage for the production Rust API.
//!
//! This case names only the seams a default-feature consumer may rely on. Its
//! negative counterparts cannot live here: every `cargo test` build resolves
//! the crate's own dev-dependency with `test-util` enabled, so a `compile_fail`
//! fixture for `TestConfig` or `with_writer_for_test()` would compile instead of
//! failing. `tests/data/rust_api_boundary/` holds those negative probes, and
//! `tests/test_release_api_boundary.py` compiles them outside this feature-
//! unified graph.

use _femtologging_rs::{
    DefaultFormatter, FemtoFileHandler, FileHandlerBuilder, HandlerConfig, OverflowPolicy,
};

fn production_handler_config() -> HandlerConfig {
    HandlerConfig {
        capacity: 8,
        flush_interval: 1,
        overflow_policy: OverflowPolicy::Drop,
    }
}

fn production_builder(path: &str) -> FileHandlerBuilder {
    FileHandlerBuilder::new(path).with_formatter(DefaultFormatter)
}

fn main() {
    let config = production_handler_config();
    assert_eq!(config.capacity, 8);
    let _ = production_builder("production.log");
    let _ = std::any::type_name::<FemtoFileHandler>();
}
