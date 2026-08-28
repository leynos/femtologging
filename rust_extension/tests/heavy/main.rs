//! Crate root for the long-running "heavy" integration tests.
//!
//! Cargo only auto-discovers `tests/*.rs` and `tests/*/main.rs`, so this file
//! must be named `main.rs` for the modules below to be compiled at all. The
//! tests it aggregates are model-checking (`loom`) and property-based suites
//! that are too slow for the ordinary gate, so each is marked `#[ignore]` and
//! run by the nightly `heavy-tests` workflow via `cargo test -- --ignored`.
//!
//! `test_utils` is declared once here and shared by the submodules through
//! `crate::test_utils`, so the shared helpers are compiled a single time for
//! this test binary.
//!
//! # The loom modules
//!
//! The `loom_*` modules are compiled unconditionally — so that they cannot
//! silently rot again — but their test functions are registered only under
//! `--cfg loom`. `FemtoStreamHandler` and `FemtoFileHandler` spawn their
//! worker with `std::thread::spawn`, and that worker then touches the
//! loom-instrumented buffer from a thread loom did not create. Loom detects
//! this and aborts the process ("cannot access Loom execution state from
//! outside a Loom model"), which would take the whole heavy binary down with
//! it. Making these models runnable requires the handlers to spawn through an
//! injectable abstraction that resolves to `loom::thread` under `--cfg loom`;
//! until then the models are kept compiling but unregistered.
#![allow(unexpected_cfgs)]

#[path = "../test_utils/mod.rs"]
mod test_utils;

mod loom_file_flush;
mod loom_push;
mod loom_topologies;
mod prop_stream_handler;
