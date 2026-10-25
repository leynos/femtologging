//! Criterion benchmarks for Rust log bridge enablement checks.

use std::hint::black_box;

use _femtologging_rs::{FemtoLevel, FemtoLogAdapter, get_logger_for_log_compat};
use criterion::{Criterion, criterion_group, criterion_main};
use log::{Level, LevelFilter, Log, Metadata};
use once_cell::sync::Lazy;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

/// Shared benchmark setup, initialized once before Criterion measures cases.
static BRIDGE: Lazy<BridgeBench> = Lazy::new(BridgeBench::initialize);

/// Adapter and metadata used by the Rust log bridge benchmark cases.
struct BridgeBench {
    /// Installed adapter whose target-aware `enabled` path is measured.
    adapter: FemtoLogAdapter,
    /// Metadata for a registered target that accepts its level.
    accepted: Metadata<'static>,
    /// Metadata for a registered target that rejects its level.
    rejected: Metadata<'static>,
    /// Metadata whose Rust module path is normalized before lookup.
    normalized: Metadata<'static>,
    /// Metadata for a valid target absent from the logger index.
    unknown: Metadata<'static>,
}

impl BridgeBench {
    /// Create loggers and install the adapter before any timed iteration.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let benchmark_setup = BridgeBench::initialize();
    /// ```
    fn initialize() -> Self {
        Python::initialize();
        let setup_result = Python::attach(|py| {
            set_logger_level(py, "bench.accepted", FemtoLevel::Trace)?;
            set_logger_level(py, "bench.rejected", FemtoLevel::Warn)?;
            set_logger_level(py, "bench.normalized", FemtoLevel::Warn)?;
            if !_femtologging_rs::install_global_logger() {
                return Err(PyRuntimeError::new_err(
                    "the log bridge should install in the standalone benchmark process",
                ));
            }
            Ok(())
        });
        if let Err(error) = setup_result {
            panic!("failed to initialize Rust log bridge benchmark: {error}");
        }

        Self {
            adapter: FemtoLogAdapter,
            accepted: metadata("bench.accepted", Level::Trace),
            rejected: metadata("bench.rejected", Level::Debug),
            normalized: metadata("bench::normalized", Level::Debug),
            unknown: metadata("bench.unknown", Level::Debug),
        }
    }
}

/// Set up a named logger threshold for one benchmark target.
///
/// # Examples
///
/// ```ignore
/// set_logger_level(py, "service.worker", FemtoLevel::Warn)?;
/// ```
fn set_logger_level(py: Python<'_>, name: &str, level: FemtoLevel) -> PyResult<()> {
    let logger = get_logger_for_log_compat(py, name)?;
    logger.borrow(py).set_level(level);
    Ok(())
}

/// Build static log metadata for a benchmark target and level.
///
/// # Examples
///
/// ```ignore
/// let metadata = metadata("service.worker", Level::Debug);
/// assert_eq!(metadata.target(), "service.worker");
/// ```
fn metadata(target: &'static str, level: Level) -> Metadata<'static> {
    Metadata::builder().target(target).level(level).build()
}

/// Construct a diagnostic payload that makes disabled argument work measurable.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(expensive_argument().split(':').count(), 128);
/// ```
fn expensive_argument() -> String {
    (0..128)
        .map(|value| format!("{value:04x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Register and run the Rust log bridge hot-path benchmark cases.
///
/// # Examples
///
/// ```ignore
/// let mut criterion = Criterion::default();
/// bench_log_bridge(&mut criterion);
/// ```
fn bench_log_bridge(c: &mut Criterion) {
    let bridge = &*BRIDGE;
    let mut group = c.benchmark_group("log_bridge_enabled");

    group.bench_function("registered_accepted", |b| {
        b.iter(|| bridge.adapter.enabled(black_box(&bridge.accepted)));
    });
    group.bench_function("registered_rejected", |b| {
        b.iter(|| bridge.adapter.enabled(black_box(&bridge.rejected)));
    });
    group.bench_function("normalized_module_target", |b| {
        b.iter(|| bridge.adapter.enabled(black_box(&bridge.normalized)));
    });
    group.bench_function("unknown_target", |b| {
        b.iter(|| bridge.adapter.enabled(black_box(&bridge.unknown)));
    });
    group.bench_function("log_enabled_macro", |b| {
        b.iter(|| log::log_enabled!(target: "bench.rejected", Level::Debug));
    });

    let level_change =
        Python::attach(|py| set_logger_level(py, "bench.accepted", FemtoLevel::Warn));
    if let Err(error) = level_change {
        panic!("failed to disable Debug in the benchmark: {error}");
    }
    assert_eq!(
        log::max_level(),
        LevelFilter::Info,
        "the expensive-argument case must be rejected by the macro prefilter",
    );

    group.bench_function("disabled_debug_expensive_argument", |b| {
        b.iter(|| {
            log::debug!(
                target: "bench.rejected",
                "diagnostic payload: {}",
                black_box(expensive_argument())
            );
        });
    });

    group.finish();
}

criterion_group!(benches, bench_log_bridge);
criterion_main!(benches);
