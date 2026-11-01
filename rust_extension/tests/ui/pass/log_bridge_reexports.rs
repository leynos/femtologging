//! Compile-pass coverage for the feature-gated Rust log bridge re-exports.
//!
//! Applications install the bridge through these two names, so the fixture
//! proves the re-export survives the `log-compat` plus `test-util` gating. It
//! only binds the names; installing the global logger is a runtime decision
//! the fixture must not make.

use _femtologging_rs::{FemtoLogAdapter, install_global_logger};

fn main() {
    let adapter = FemtoLogAdapter;
    let _logger: &dyn log::Log = &adapter;
    let _install: fn() -> bool = install_global_logger;
}
