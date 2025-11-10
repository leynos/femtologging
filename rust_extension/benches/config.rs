#![cfg(feature = "python")]

use std::path::PathBuf;

use _femtologging_rs::{
    manager, ConfigBuilder, FemtoLevel, LoggerConfigBuilder, StreamHandlerBuilder,
};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

struct PythonApis {
    reset_manager: Py<PyAny>,
    basic_config: Py<PyAny>,
    dict_config: Py<PyAny>,
    stdout: Py<PyAny>,
    dict_schema: Py<PyAny>,
}

impl PythonApis {
    fn new() -> Self {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let sys = py.import("sys").expect("import sys");
            let path: &PyList = sys
                .getattr("path")
                .and_then(|obj| obj.downcast::<PyList>())
                .expect("sys.path should be a list");
            let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("workspace root")
                .to_path_buf();
            let repo_str = repo_root.to_string_lossy();
            path.insert(0, repo_str.as_ref())
                .expect("inject workspace root into sys.path");

            let femto = py.import("femtologging").expect("import femtologging");
            let config_mod = py
                .import("femtologging.config")
                .expect("import femtologging.config");
            let reset_manager = femto.getattr("reset_manager").unwrap().into_py(py);
            let basic_config = femto.getattr("basicConfig").unwrap().into_py(py);
            let dict_config = config_mod.getattr("dictConfig").unwrap().into_py(py);
            let stdout = sys.getattr("stdout").unwrap().into_py(py);

            let locals = PyDict::new(py);
            py.run(
                r#"
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
"#,
                None,
                Some(locals),
            )
            .expect("build dictConfig schema");
            let dict_schema = locals.get_item("schema").unwrap().to_object(py);

            Self {
                reset_manager,
                basic_config,
                dict_config,
                stdout,
                dict_schema,
            }
        })
    }

    fn reset(&self, py: Python<'_>) {
        self.reset_manager.call0(py).expect("reset_manager()");
    }

    fn run_basic_config(&self, py: Python<'_>) {
        let kwargs = PyDict::new(py);
        kwargs.set_item("level", "INFO").unwrap();
        kwargs
            .set_item("stream", self.stdout.as_ref(py))
            .expect("set stream");
        kwargs.set_item("force", true).unwrap();
        self.basic_config
            .call(py, (), Some(kwargs))
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
    let apis = PythonApis::new();
    let mut group = c.benchmark_group("configuration");

    group.bench_function("builder_build_and_init", |b| {
        b.iter(|| {
            manager::reset_manager();
            let builder = sample_builder();
            black_box(&builder)
                .build_and_init()
                .expect("builder build_and_init()");
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
