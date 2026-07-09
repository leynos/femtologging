//! Benchmarks for femtologging configuration flows under the `python` feature.
#![cfg(feature = "python")]

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use _femtologging_rs::{
    ConfigBuilder, FemtoLevel, LoggerConfigBuilder, StreamHandlerBuilder, manager,
};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use once_cell::sync::Lazy;
use pyo3::{
    exceptions::PyKeyError,
    prelude::*,
    types::{PyAny, PyDict, PyList},
};

const DICT_SCHEMA_PY: &std::ffi::CStr = cr#"
from femtologging import ConfigBuilder, LoggerConfigBuilder, StreamHandlerBuilder

_schema_builder = (
    ConfigBuilder()
    .with_handler("console", StreamHandlerBuilder.stderr())
    .with_root_logger(
        LoggerConfigBuilder()
        .with_level("INFO")
        .with_handlers(["console"])
    )
)
schema = _schema_builder.as_dict()
"#;

static PY_APIS: Lazy<PythonApis> = Lazy::new(|| {
    Python::initialize();
    // A bench has no harness to report a Result to, so surface setup failure
    // with an explicit panic at this single boundary.
    match Python::attach(PythonApis::new) {
        Ok(apis) => apis,
        Err(err) => panic!("failed to initialize Python benchmark APIs: {err}"),
    }
});

struct PythonApis {
    reset_manager: Py<PyAny>,
    basic_config: Py<PyAny>,
    dict_config: Py<PyAny>,
    stdout: Py<PyAny>,
    dict_schema: Py<PyAny>,
}

impl PythonApis {
    fn new(py: Python<'_>) -> PyResult<Self> {
        let (reset_manager, basic_config, dict_config, stdout) = init_python_imports(py)?;
        let dict_schema = build_dict_schema(py)?;
        Ok(Self {
            reset_manager,
            basic_config,
            dict_config,
            stdout,
            dict_schema,
        })
    }

    fn reset(&self, py: Python<'_>) -> PyResult<()> {
        self.reset_manager.call0(py)?;
        Ok(())
    }

    fn run_basic_config(&self, py: Python<'_>) -> PyResult<()> {
        let kwargs = PyDict::new(py);
        kwargs.set_item("level", "INFO")?;
        let stdout = self.stdout.bind(py);
        kwargs.set_item("stream", stdout)?;
        kwargs.set_item("force", true)?;
        self.basic_config.call(py, (), Some(&kwargs))?;
        Ok(())
    }

    fn run_dict_config(&self, py: Python<'_>) -> PyResult<()> {
        let schema_copy = self.dict_schema.call_method0(py, "copy")?;
        self.dict_config.call1(py, (schema_copy,))?;
        Ok(())
    }
}

fn init_python_imports(py: Python<'_>) -> PyResult<(Py<PyAny>, Py<PyAny>, Py<PyAny>, Py<PyAny>)> {
    let sys = py.import("sys")?;
    let sys_any = sys.as_any();
    inject_repo_to_path(&sys_any)?;
    let femto = py.import("femtologging")?;
    let config_mod = py.import("femtologging.config")?;
    let reset_manager = femto.getattr("reset_manager")?.unbind();
    let basic_config = femto.getattr("basicConfig")?.unbind();
    let dict_config = config_mod.getattr("dictConfig")?.unbind();
    let stdout = sys.getattr("stdout")?.unbind();
    Ok((reset_manager, basic_config, dict_config, stdout))
}

fn inject_repo_to_path(sys: &Bound<'_, PyAny>) -> PyResult<()> {
    let path = sys.getattr("path")?;
    let path = path.cast::<PyList>()?;
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| PyKeyError::new_err("manifest dir has no parent"))?;
    let repo_str = repo_root.to_string_lossy().into_owned();
    path.insert(0, repo_str.as_str())?;
    Ok(())
}

fn build_dict_schema(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let locals = PyDict::new(py);
    py.run(DICT_SCHEMA_PY, None, Some(&locals))?;
    locals
        .get_item("schema")?
        .ok_or_else(|| PyKeyError::new_err("schema must be defined by the setup script"))
        .map(pyo3::Bound::unbind)
}

fn sample_builder() -> ConfigBuilder {
    let worker = LoggerConfigBuilder::new().with_handlers(["console"]);
    let audit = worker.clone().with_propagate(false);
    ConfigBuilder::new()
        .with_handler("console", StreamHandlerBuilder::stderr())
        .with_default_level(FemtoLevel::Info)
        .with_logger("service.worker", worker)
        .with_logger("service.audit", audit)
        .with_root_logger(
            LoggerConfigBuilder::new()
                .with_level(FemtoLevel::Warn)
                .with_handlers(["console"]),
        )
}

/// Consume a benchmark-iteration Result, panicking with context on failure.
///
/// Criterion closures cannot propagate errors, so this is the sanctioned
/// panic boundary for the bench.
fn expect_bench<T, E: std::fmt::Display>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(err) => panic!("{context}: {err}"),
    }
}

fn config_benchmarks(c: &mut Criterion) {
    manager::reset_manager();
    let apis = &*PY_APIS;
    let mut group = c.benchmark_group("configuration");

    group.bench_function("builder_build_and_init", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                manager::reset_manager();
                let start = Instant::now();
                let builder = sample_builder();
                expect_bench(
                    black_box(&builder).build_and_init(),
                    "builder build_and_init()",
                );
                total += start.elapsed();
            }
            total
        });
    });

    group.bench_function("basicConfig_stream_stdout", |b| {
        b.iter(|| {
            Python::attach(|py| {
                expect_bench(apis.reset(py), "reset_manager()");
                expect_bench(apis.run_basic_config(py), "basicConfig()");
            });
        });
    });

    group.bench_function("dictConfig_round_trip", |b| {
        b.iter(|| {
            Python::attach(|py| {
                expect_bench(apis.reset(py), "reset_manager()");
                expect_bench(apis.run_dict_config(py), "dictConfig()");
            });
        });
    });

    group.finish();
}

criterion_group!(benches, config_benchmarks);
criterion_main!(benches);
