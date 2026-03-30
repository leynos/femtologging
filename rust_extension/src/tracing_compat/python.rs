//! Python-facing helpers for the tracing compatibility bridge.

use std::sync::OnceLock;

use pyo3::prelude::*;
use tracing_subscriber::prelude::*;

use crate::level::FemtoLevel;

use super::layer;

static INSTALL_RESULT: OnceLock<bool> = OnceLock::new();

fn install_global_tracing_subscriber() -> bool {
    *INSTALL_RESULT.get_or_init(|| {
        let subscriber = tracing_subscriber::registry().with(layer());
        tracing::subscriber::set_global_default(subscriber).is_ok()
    })
}

fn emit_message_event(level: FemtoLevel, message: &str) {
    match level {
        FemtoLevel::Trace => {
            tracing::event!(target: "rust.tracing.basic", tracing::Level::TRACE, "{message}")
        }
        FemtoLevel::Debug => {
            tracing::event!(target: "rust.tracing.basic", tracing::Level::DEBUG, "{message}")
        }
        FemtoLevel::Info => {
            tracing::event!(target: "rust.tracing.basic", tracing::Level::INFO, "{message}")
        }
        FemtoLevel::Warn => {
            tracing::event!(target: "rust.tracing.basic", tracing::Level::WARN, "{message}")
        }
        FemtoLevel::Error | FemtoLevel::Critical => {
            tracing::event!(target: "rust.tracing.basic", tracing::Level::ERROR, "{message}")
        }
    }
}

/// Install a process-global tracing subscriber backed by femtologging.
///
/// The helper is explicit and idempotent, mirroring `setup_rust_logging()`.
#[pyfunction]
pub(crate) fn setup_rust_tracing() -> PyResult<()> {
    if install_global_tracing_subscriber() {
        Ok(())
    } else {
        Err(pyo3::exceptions::PyRuntimeError::new_err(
            "global tracing subscriber is already set; femtologging cannot install the tracing bridge",
        ))
    }
}

/// Emit a basic tracing event for behavioural tests.
#[pyfunction]
#[pyo3(name = "_emit_rust_tracing_event")]
pub(crate) fn emit_rust_tracing_event(level: FemtoLevel, message: &str) {
    emit_message_event(level, message);
}

/// Emit a tracing event carrying structured fields for behavioural tests.
#[pyfunction]
#[pyo3(name = "_emit_rust_tracing_structured_event")]
pub(crate) fn emit_rust_tracing_structured_event() {
    tracing::info!(
        target: "rust.tracing.structured",
        user_id = 42_u64,
        success = true,
        latency_ms = 12.5_f64,
        details = ?vec!["alpha", "beta"],
        "structured event"
    );
}

/// Emit a tracing event inside nested spans for behavioural tests.
#[pyfunction]
#[pyo3(name = "_emit_rust_tracing_span_event")]
pub(crate) fn emit_rust_tracing_span_event() {
    let outer = tracing::info_span!(target: "rust.tracing.span", "request", request_id = "req-42");
    let _outer_guard = outer.enter();

    let inner = tracing::info_span!(target: "rust.tracing.span", "step", attempt = 2_u64);
    let _inner_guard = inner.enter();

    tracing::info!(target: "rust.tracing.span", success = true, "span event");
}

/// Install a different global tracing subscriber for subprocess failure tests.
#[pyfunction]
#[pyo3(name = "_install_test_global_tracing_subscriber")]
pub(crate) fn install_test_global_tracing_subscriber() -> PyResult<()> {
    tracing::subscriber::set_global_default(tracing_subscriber::registry()).map_err(|_| {
        pyo3::exceptions::PyRuntimeError::new_err("global tracing subscriber is already set")
    })
}
