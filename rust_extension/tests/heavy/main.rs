//! Crate root for the long-running "heavy" integration tests.
//!
//! Cargo only auto-discovers `tests/*.rs` and `tests/*/main.rs`, so this file
//! must be named `main.rs` for the modules below to be compiled at all. The
//! tests it aggregates are model-checking (`loom`) and property-based suites
//! that are too slow for the ordinary gate. The property suite is marked
//! `#[ignore]` and run by the nightly `heavy-tests` workflow via
//! `cargo test -- --ignored`; the loom models are gated by configuration
//! instead, as described below.
//!
//! Shared buffer support is declared once here and shared by the property suite
//! and, when enabled, the loom models. The `HandleExpect` helper is shared by
//! the property suite and the loom models.
//!
//! # The loom modules
//!
//! The `loom_*` modules compile only under `--cfg loom`. The handlers and the
//! logger spawn, queue and lock through the crate's concurrency seam, which
//! resolves to Loom's primitives in that configuration, so the models run the
//! production workers inside the model. The heavy workflow runs them with
//! `--include-ignored` under `scripts/run_loom_models.py`, which requires every
//! model in `loom-models.txt` to pass; see "Loom models" in
//! `docs/developers-guide.md`.

#[path = "../test_utils/shared_buffer.rs"]
mod shared_buffer;

#[path = "../test_utils/handle_expect.rs"]
mod handle_expect;

#[cfg(loom)]
mod loom_file_flush;
#[cfg(loom)]
mod loom_push;
#[cfg(loom)]
mod loom_sink;
#[cfg(loom)]
mod loom_topologies;
mod prop_stream_handler;
