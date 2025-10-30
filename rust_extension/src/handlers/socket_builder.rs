//! Builder for [`FemtoSocketHandler`](crate::socket_handler::FemtoSocketHandler).
//!
//! Exposes transport selection, timeout tuning, TLS configuration, and
//! exponential backoff parameters. The builder mirrors configuration concepts
//! described in the design documents to keep the Python and Rust APIs aligned.

use std::{path::PathBuf, time::Duration};

#[cfg(feature = "python")]
use pyo3::{prelude::*, types::PyDict, Bound};

#[cfg(feature = "python")]
use crate::macros::{dict_into_py, AsPyDict};

use crate::socket_handler::{
    BackoffPolicy, FemtoSocketHandler, SocketHandlerConfig, SocketTransport, TcpTransport,
    TlsOptions, UnixTransport,
};

use super::{HandlerBuildError, HandlerBuilderTrait};

#[derive(Clone, Debug)]
enum TransportConfig {
    Tcp { host: String, port: u16 },
    Unix { path: PathBuf },
}

#[derive(Clone, Debug, Default)]
struct TlsConfig {
    domain: Option<String>,
    insecure: bool,
}

#[derive(Clone, Debug, Default)]
struct BackoffConfig {
    base_ms: Option<u64>,
    cap_ms: Option<u64>,
    reset_after_ms: Option<u64>,
    deadline_ms: Option<u64>,
}

impl BackoffConfig {
    fn apply(&self, policy: &mut BackoffPolicy) -> Result<(), HandlerBuildError> {
        if let Some(base) = self.base_ms {
            ensure_positive_u64(base, "backoff_base_ms")?;
            policy.base = Duration::from_millis(base);
        }
        if let Some(cap) = self.cap_ms {
            ensure_positive_u64(cap, "backoff_cap_ms")?;
            policy.cap = Duration::from_millis(cap);
        }
        if let Some(reset) = self.reset_after_ms {
            ensure_positive_u64(reset, "backoff_reset_after_ms")?;
            policy.reset_after = Duration::from_millis(reset);
        }
        if let Some(deadline) = self.deadline_ms {
            ensure_positive_u64(deadline, "backoff_deadline_ms")?;
            policy.deadline = Duration::from_millis(deadline);
        }
        Ok(())
    }
}

fn ensure_positive_u64(value: u64, field: &str) -> Result<u64, HandlerBuildError> {
    if value == 0 {
        Err(HandlerBuildError::InvalidConfig(format!(
            "{field} must be greater than zero"
        )))
    } else {
        Ok(value)
    }
}

fn ensure_positive_usize(value: usize, field: &str) -> Result<usize, HandlerBuildError> {
    if value == 0 {
        Err(HandlerBuildError::InvalidConfig(format!(
            "{field} must be greater than zero"
        )))
    } else {
        Ok(value)
    }
}

macro_rules! option_setter {
    ($fn_name:ident, $field:ident, $ty:ty) => {
        pub fn $fn_name(mut self, value: $ty) -> Self {
            self.$field = Some(value);
            self
        }
    };
}

#[cfg(feature = "python")]
macro_rules! dict_set {
    ($dict:expr, $key:expr, $opt:expr) => {
        if let Some(value) = $opt {
            $dict.set_item($key, value)?;
        }
    };
}

/// Builder for constructing [`FemtoSocketHandler`] instances.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Clone, Debug, Default)]
pub struct SocketHandlerBuilder {
    capacity: Option<usize>,
    connect_timeout_ms: Option<u64>,
    write_timeout_ms: Option<u64>,
    max_frame_size: Option<usize>,
    transport: Option<TransportConfig>,
    tls: Option<TlsConfig>,
    backoff: BackoffConfig,
}

impl SocketHandlerBuilder {
    /// Create a new builder with no transport configured.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure the builder to use TCP.
    pub fn with_tcp(mut self, host: impl Into<String>, port: u16) -> Self {
        self.transport = Some(TransportConfig::Tcp {
            host: host.into(),
            port,
        });
        self
    }

    /// Configure the builder to use a Unix domain socket.
    pub fn with_unix_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.transport = Some(TransportConfig::Unix { path: path.into() });
        self
    }

    /// Configure TLS using the provided domain and validation policy.
    pub fn with_tls(mut self, domain: Option<String>, insecure: bool) -> Self {
        self.tls = Some(TlsConfig { domain, insecure });
        self
    }

    /// Set the bounded channel capacity.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = Some(capacity);
        self
    }

    option_setter!(with_connect_timeout_ms, connect_timeout_ms, u64);
    option_setter!(with_write_timeout_ms, write_timeout_ms, u64);
    option_setter!(with_max_frame_size, max_frame_size, usize);

    /// Override backoff timings using milliseconds.
    pub fn with_backoff(
        mut self,
        base_ms: Option<u64>,
        cap_ms: Option<u64>,
        reset_after_ms: Option<u64>,
        deadline_ms: Option<u64>,
    ) -> Self {
        self.backoff = BackoffConfig {
            base_ms,
            cap_ms,
            reset_after_ms,
            deadline_ms,
        };
        self
    }

    fn validate(&self) -> Result<(), HandlerBuildError> {
        self.validate_transport()?;
        self.validate_capacity()?;
        self.validate_timeouts()?;
        self.validate_frame_size()?;
        Ok(())
    }

    fn validate_transport(&self) -> Result<(), HandlerBuildError> {
        match &self.transport {
            None => Err(HandlerBuildError::InvalidConfig(
                "socket handler requires a transport".into(),
            )),
            Some(TransportConfig::Unix { .. }) if self.tls.is_some() => Err(
                HandlerBuildError::InvalidConfig("tls is only supported for tcp transports".into()),
            ),
            _ => Ok(()),
        }
    }

    fn validate_capacity(&self) -> Result<(), HandlerBuildError> {
        if let Some(capacity) = self.capacity {
            ensure_positive_usize(capacity, "capacity")?;
        }
        Ok(())
    }

    fn validate_timeouts(&self) -> Result<(), HandlerBuildError> {
        if let Some(timeout) = self.connect_timeout_ms {
            ensure_positive_u64(timeout, "connect_timeout_ms")?;
        }
        if let Some(timeout) = self.write_timeout_ms {
            ensure_positive_u64(timeout, "write_timeout_ms")?;
        }
        Ok(())
    }

    fn validate_frame_size(&self) -> Result<(), HandlerBuildError> {
        if let Some(size) = self.max_frame_size {
            ensure_positive_usize(size, "max_frame_size")?;
        }
        Ok(())
    }

    fn build_config(&self) -> Result<SocketHandlerConfig, HandlerBuildError> {
        self.validate()?;
        let mut config = SocketHandlerConfig::default();
        if let Some(capacity) = self.capacity {
            config.capacity = ensure_positive_usize(capacity, "capacity")?;
        }
        if let Some(timeout) = self.connect_timeout_ms {
            config.connect_timeout =
                Duration::from_millis(ensure_positive_u64(timeout, "connect_timeout_ms")?);
        }
        if let Some(timeout) = self.write_timeout_ms {
            config.write_timeout =
                Duration::from_millis(ensure_positive_u64(timeout, "write_timeout_ms")?);
        }
        if let Some(size) = self.max_frame_size {
            config.max_frame_size = ensure_positive_usize(size, "max_frame_size")?;
        }
        if let Some(ref transport) = self.transport {
            config.transport = match transport {
                TransportConfig::Tcp { host, port } => {
                    if host.trim().is_empty() {
                        return Err(HandlerBuildError::InvalidConfig(
                            "tcp host must not be empty".into(),
                        ));
                    }
                    let tls_options = self.tls.as_ref().map(|tls_cfg| {
                        let domain = tls_cfg
                            .domain
                            .clone()
                            .and_then(|d| if d.trim().is_empty() { None } else { Some(d) })
                            .unwrap_or_else(|| host.clone());
                        TlsOptions {
                            domain,
                            insecure_skip_verify: tls_cfg.insecure,
                        }
                    });
                    SocketTransport::Tcp(TcpTransport {
                        host: host.clone(),
                        port: *port,
                        tls: tls_options,
                    })
                }
                TransportConfig::Unix { path } => {
                    SocketTransport::Unix(UnixTransport { path: path.clone() })
                }
            };
        }
        self.backoff.apply(&mut config.backoff)?;
        Ok(config)
    }

    #[cfg(feature = "python")]
    fn extend_dict(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        dict_set!(d, "capacity", self.capacity);
        dict_set!(d, "connect_timeout_ms", self.connect_timeout_ms);
        dict_set!(d, "write_timeout_ms", self.write_timeout_ms);
        dict_set!(d, "max_frame_size", self.max_frame_size);
        match &self.transport {
            Some(TransportConfig::Tcp { host, port }) => {
                d.set_item("transport", "tcp")?;
                d.set_item("host", host)?;
                d.set_item("port", *port)?;
                if let Some(tls_cfg) = &self.tls {
                    d.set_item("tls", true)?;
                    if let Some(domain) = &tls_cfg.domain {
                        d.set_item("tls_domain", domain)?;
                    }
                    d.set_item("tls_insecure", tls_cfg.insecure)?;
                } else {
                    d.set_item("tls", false)?;
                }
            }
            Some(TransportConfig::Unix { path }) => {
                d.set_item("transport", "unix")?;
                d.set_item("path", path.display().to_string())?;
            }
            None => {}
        }
        dict_set!(d, "backoff_base_ms", self.backoff.base_ms);
        dict_set!(d, "backoff_cap_ms", self.backoff.cap_ms);
        dict_set!(d, "backoff_reset_after_ms", self.backoff.reset_after_ms);
        dict_set!(d, "backoff_deadline_ms", self.backoff.deadline_ms);
        Ok(())
    }
}

impl HandlerBuilderTrait for SocketHandlerBuilder {
    type Handler = FemtoSocketHandler;

    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError> {
        let config = self.build_config()?;
        Ok(FemtoSocketHandler::with_config(config))
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl SocketHandlerBuilder {
    #[new]
    fn py_new() -> PyResult<Self> {
        Ok(Self::new())
    }

    #[pyo3(name = "with_tcp")]
    #[pyo3(signature = (host, port))]
    fn py_with_tcp<'py>(
        mut slf: PyRefMut<'py, Self>,
        host: String,
        port: u16,
    ) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_tcp(host, port);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_unix_path")]
    #[pyo3(signature = (path))]
    fn py_with_unix_path<'py>(mut slf: PyRefMut<'py, Self>, path: String) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_unix_path(path);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_capacity")]
    #[pyo3(signature = (capacity))]
    fn py_with_capacity<'py>(
        mut slf: PyRefMut<'py, Self>,
        capacity: usize,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let updated = slf.clone().with_capacity(capacity);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_connect_timeout_ms")]
    #[pyo3(signature = (timeout_ms))]
    fn py_with_connect_timeout<'py>(
        mut slf: PyRefMut<'py, Self>,
        timeout_ms: u64,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let updated = slf.clone().with_connect_timeout_ms(timeout_ms);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_write_timeout_ms")]
    #[pyo3(signature = (timeout_ms))]
    fn py_with_write_timeout<'py>(
        mut slf: PyRefMut<'py, Self>,
        timeout_ms: u64,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let updated = slf.clone().with_write_timeout_ms(timeout_ms);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_max_frame_size")]
    #[pyo3(signature = (size))]
    fn py_with_max_frame_size<'py>(
        mut slf: PyRefMut<'py, Self>,
        size: u64,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let size = usize::try_from(size).map_err(|_| {
            pyo3::exceptions::PyOverflowError::new_err(
                "max_frame_size does not fit in platform usize",
            )
        })?;
        let updated = slf.clone().with_max_frame_size(size);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_tls")]
    #[pyo3(signature = (domain=None, *, insecure=false))]
    fn py_with_tls<'py>(
        mut slf: PyRefMut<'py, Self>,
        domain: Option<String>,
        insecure: bool,
    ) -> PyRefMut<'py, Self> {
        let updated = slf.clone().with_tls(domain, insecure);
        *slf = updated;
        slf
    }

    #[pyo3(name = "with_backoff")]
    #[pyo3(signature = (base_ms=None, cap_ms=None, reset_after_ms=None, deadline_ms=None))]
    fn py_with_backoff<'py>(
        mut slf: PyRefMut<'py, Self>,
        base_ms: Option<u64>,
        cap_ms: Option<u64>,
        reset_after_ms: Option<u64>,
        deadline_ms: Option<u64>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let updated = slf
            .clone()
            .with_backoff(base_ms, cap_ms, reset_after_ms, deadline_ms);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "as_dict")]
    fn py_as_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let dict = PyDict::new(py);
        self.extend_dict(&dict)?;
        Ok(dict.into())
    }

    #[pyo3(name = "build")]
    fn py_build(&self) -> PyResult<FemtoSocketHandler> {
        self.build_inner().map_err(Into::into)
    }
}

#[cfg(feature = "python")]
impl AsPyDict for SocketHandlerBuilder {
    fn as_pydict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = PyDict::new(py);
        self.extend_dict(&d)?;
        dict_into_py(d, py)
    }
}
