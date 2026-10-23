//! Criterion benchmarks for Rust log bridge enablement checks.

use std::hint::black_box;

use _femtologging_rs::{FemtoLevel, FemtoLogAdapter, get_logger_for_log_compat};
use criterion::{Criterion, criterion_group, criterion_main};
use log::{Level, Log, Metadata};
use once_cell::sync::Lazy;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

static BRIDGE: Lazy<BridgeBench> = Lazy::new(BridgeBench::initialize);

struct BridgeBench {
    adapter: FemtoLogAdapter,
    accepted: Metadata<'static>,
    rejected: Metadata<'static>,
    normalized: Metadata<'static>,
    unknown: Metadata<'static>,
}

impl BridgeBench {
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

fn set_logger_level(py: Python<'_>, name: &str, level: FemtoLevel) -> PyResult<()> {
    let logger = get_logger_for_log_compat(py, name)?;
    logger.borrow(py).set_level(level);
    Ok(())
}

fn metadata(target: &'static str, level: Level) -> Metadata<'static> {
    Metadata::builder().target(target).level(level).build()
}

fn expensive_argument() -> String {
    (0..128)
        .map(|value| format!("{value:04x}"))
        .collect::<Vec<_>>()
        .join(":")
}

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
