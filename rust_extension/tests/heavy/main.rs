//! Crate root for the long-running "heavy" integration tests.
//!
//! Cargo only auto-discovers `tests/*.rs` and `tests/*/main.rs`, so this file
//! must be named `main.rs` for the modules below to be compiled at all. The
//! tests it aggregates are model-checking (`loom`) and property-based suites
//! that are too slow for the ordinary gate, so each is marked `#[ignore]` and
//! run by the nightly `heavy-tests` workflow via `cargo test -- --ignored`.
//!
//! Shared buffer support is declared once here and shared by the property suite
//! and, when enabled, the loom models. The `HandleExpect` helper is shared by
//! the property suite and the loom models.
//!
//! # The loom modules
//!
//! The `loom_*` modules compile only under `--cfg loom`. `FemtoStreamHandler`
//! and `FemtoFileHandler` spawn their worker with `std::thread::spawn`, and
//! that worker then touches the loom-instrumented buffer from a thread loom did
//! not create. Loom detects this and aborts the process ("cannot access Loom
//! execution state from outside a Loom model"), which would take the whole
//! heavy binary down with it. Making these models runnable requires the
//! handlers to spawn through an injectable abstraction that resolves to
//! `loom::thread` under `--cfg loom`; until then the models are compile-checked
//! but not run by the heavy workflow.

#[path = "../test_utils/shared_buffer.rs"]
mod shared_buffer;

#[path = "../test_utils/handle_expect.rs"]
mod handle_expect;

#[cfg(loom)]
mod loom_file_flush;
#[cfg(loom)]
mod loom_push;
#[cfg(loom)]
mod loom_topologies;
mod prop_stream_handler;
