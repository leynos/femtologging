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
    prelude::*,
    types::{PyAny, PyDict, PyList},
};

const DICT_SCHEMA_PY: &str = r#"
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
    pyo3::prepare_freethreaded_python();
    Python::with_gil(|py| PythonApis::new(py))
});

struct PythonApis {
    reset_manager: Py<PyAny>,
    basic_config: Py<PyAny>,
    dict_config: Py<PyAny>,
    stdout: Py<PyAny>,
    dict_schema: Py<PyAny>,
}

impl PythonApis {
    fn new(py: Python<'_>) -> Self {
        let (reset_manager, basic_config, dict_config, stdout) = init_python_imports(py);
        let dict_schema = build_dict_schema(py);
        Self {
            reset_manager,
            basic_config,
            dict_config,
            stdout,
            dict_schema,
        }
    }

    fn reset(&self, py: Python<'_>) {
        self.reset_manager.call0(py).expect("reset_manager()");
    }

    fn run_basic_config(&self, py: Python<'_>) {
        let kwargs = PyDict::new_bound(py);
        kwargs
            .set_item("level", "INFO")
            .expect("set level for basicConfig");
        let stdout = self.stdout.bind(py);
        kwargs.set_item("stream", stdout).expect("set stream");
        kwargs
            .set_item("force", true)
            .expect("set force for basicConfig");
        self.basic_config
            .call(py, (), Some(&kwargs))
            .expect("basicConfig()");
    }

    fn run_dict_config(&self, py: Python<'_>) {
        let schema_copy = self
            .dict_schema
            .call_method0(py, "copy")
            .expect("schema.copy()");
        self.dict_config
            .call1(py, (schema_copy,))
            .expect("dictConfig()");
    }
}

fn init_python_imports(py: Python<'_>) -> (Py<PyAny>, Py<PyAny>, Py<PyAny>, Py<PyAny>) {
    let sys = py.import("sys").expect("import sys");
    let sys_any = sys.as_any();
    inject_repo_to_path(&sys_any);
    let femto = py.import("femtologging").expect("import femtologging");
    let config_mod = py
        .import("femtologging.config")
        .expect("import femtologging.config");
    let reset_manager = femto
        .getattr("reset_manager")
        .expect("femtologging.reset_manager attr")
        .unbind();
    let basic_config = femto
        .getattr("basicConfig")
        .expect("femtologging.basicConfig attr")
        .unbind();
    let dict_config = config_mod
        .getattr("dictConfig")
        .expect("femtologging.config.dictConfig attr")
        .unbind();
    let stdout = sys.getattr("stdout").expect("sys.stdout attr").unbind();
    (reset_manager, basic_config, dict_config, stdout)
}

fn inject_repo_to_path(sys: &Bound<'_, PyAny>) {
    let path = sys.getattr("path").expect("sys.path attribute");
    let path: Bound<'_, PyList> = path.downcast().expect("sys.path should downcast to PyList");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let repo_str = repo_root.to_string_lossy().into_owned();
    path.insert(0, repo_str.as_str())
        .expect("insert workspace root into sys.path");
}

fn build_dict_schema(py: Python<'_>) -> Py<PyAny> {
    let locals = PyDict::new(py);
    py.run(DICT_SCHEMA_PY, None, Some(locals))
        .expect("build dictConfig schema");
    locals.get_item("schema").unwrap().to_object(py)
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
                black_box(&builder)
                    .build_and_init()
                    .expect("builder build_and_init()");
                total += start.elapsed();
            }
            total
        });
    });

    group.bench_function("basicConfig_stream_stdout", |b| {
        b.iter(|| {
            Python::with_gil(|py| {
                apis.reset(py);
                apis.run_basic_config(py);
            });
        });
    });

    group.bench_function("dictConfig_round_trip", |b| {
        b.iter(|| {
            Python::with_gil(|py| {
                apis.reset(py);
                apis.run_dict_config(py);
            });
        });
    });

    group.finish();
}

criterion_group!(benches, config_benchmarks);
criterion_main!(benches);
