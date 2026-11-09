//! Bounded diagnostics for the Python-facing file-handler waits.
//!
//! The three Python entry points that may block — `handle()`, `flush()`, and
//! `close()` — record one counter per outcome and accumulate their wait
//! duration. A gauge counts the waits in flight, so a caller can see that a
//! wait is blocked *while* it is blocked rather than only after the fact. The
//! counter set is closed: it never carries logger names, paths, messages, or
//! raw timeout values, so an operator can scrape it without unbounded
//! cardinality. `FemtoFileHandler.wait_diagnostics()` exposes a snapshot as a
//! Python dictionary, and each record also emits an internal `tracing` event
//! when the `tracing-compat` feature is enabled.
//!
//! Events target `femtologging::internal`, which `FemtoTracingLayer` ignores so
//! that instrumentation cannot recursively log about itself. Every field is
//! bounded: operation, handler kind, policy bucket, outcome, and elapsed
//! microseconds.
//!
//! The module is split along the three responsibilities so no one file carries
//! the whole surface: [`outcome`] owns the closed outcome vocabulary,
//! [`diagnostics`] owns the counters and the snapshot, and [`timer`] owns the
//! timing and the event contract.

mod diagnostics;
mod outcome;
mod timer;

pub(crate) use diagnostics::WaitDiagnostics;
pub(crate) use outcome::WaitOutcome;
pub(crate) use timer::{WaitContext, WaitSite, WaitTimer};

#[cfg(test)]
mod tests;
