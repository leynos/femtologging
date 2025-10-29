//! Builder for [`FemtoSocketHandler`](crate::socket_handler::FemtoSocketHandler).
//!
//! Exposes transport selection, timeout tuning, TLS configuration, and
//! exponential backoff parameters. The builder mirrors configuration concepts
//! described in the design documents to keep the Python and Rust APIs aligned.

use std::{
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
    time::Duration,
};

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
    Tcp {
        host: String,
        port: u16,
        tls: Option<TlsConfig>,
    },
    Unix {
        path: PathBuf,
    },
}

#[derive(Clone, Debug)]
struct TlsConfig {
    domain: Option<String>,
    insecure: bool,
}

#[derive(Clone, Debug, Default)]
struct BackoffConfig {
    base_ms: Option<NonZeroU64>,
    cap_ms: Option<NonZeroU64>,
    reset_after_ms: Option<NonZeroU64>,
    deadline_ms: Option<NonZeroU64>,
}

impl BackoffConfig {
    fn apply(&self, policy: &mut BackoffPolicy) {
        if let Some(base) = self.base_ms {
            policy.base = Duration::from_millis(base.get());
        }
        if let Some(cap) = self.cap_ms {
            policy.cap = Duration::from_millis(cap.get());
        }
        if let Some(reset) = self.reset_after_ms {
            policy.reset_after = Duration::from_millis(reset.get());
        }
        if let Some(deadline) = self.deadline_ms {
            policy.deadline = Duration::from_millis(deadline.get());
        }
    }
}

/// Builder for constructing [`FemtoSocketHandler`] instances.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Clone, Debug, Default)]
pub struct SocketHandlerBuilder {
    capacity: Option<NonZeroUsize>,
    capacity_set: bool,
    connect_timeout_ms: Option<NonZeroU64>,
    write_timeout_ms: Option<NonZeroU64>,
    max_frame_size: Option<NonZeroUsize>,
    transport: Option<TransportConfig>,
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
            tls: None,
        });
        self
    }

    /// Configure the builder to use a Unix domain socket.
    pub fn with_unix_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.transport = Some(TransportConfig::Unix { path: path.into() });
        self
    }

    /// Enable TLS using the provided domain. Defaults to the TCP host when omitted.
    pub fn with_tls_domain(mut self, domain: Option<String>) -> Self {
        self.transport = match self.transport.take() {
            Some(TransportConfig::Tcp { host, port, tls }) => {
                let mut tls_config = tls.unwrap_or(TlsConfig {
                    domain: None,
                    insecure: false,
                });
                tls_config.domain = domain;
                Some(TransportConfig::Tcp {
                    host,
                    port,
                    tls: Some(tls_config),
                })
            }
            other => other,
        };
        self
    }

    /// Toggle TLS certificate validation.
    pub fn with_insecure_tls(mut self, insecure: bool) -> Self {
        self.transport = match self.transport.take() {
            Some(TransportConfig::Tcp { host, port, tls }) => {
                let mut tls_config = tls.unwrap_or(TlsConfig {
                    domain: None,
                    insecure: false,
                });
                tls_config.insecure = insecure;
                Some(TransportConfig::Tcp {
                    host,
                    port,
                    tls: Some(tls_config),
                })
            }
            other => other,
        };
        self
    }

    /// Set the bounded channel capacity.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        if capacity == 0 {
            self.capacity = None;
            self.capacity_set = true;
        } else {
            self.capacity = Some(
                NonZeroUsize::new(capacity)
                    .expect("NonZeroUsize::new must succeed for non-zero capacity"),
            );
            self.capacity_set = true;
        }
        self
    }

    /// Set the socket connect timeout in milliseconds.
    pub fn with_connect_timeout_ms(mut self, timeout: NonZeroU64) -> Self {
        self.connect_timeout_ms = Some(timeout);
        self
    }

    /// Set the socket write timeout in milliseconds.
    pub fn with_write_timeout_ms(mut self, timeout: NonZeroU64) -> Self {
        self.write_timeout_ms = Some(timeout);
        self
    }

    /// Set the maximum allowed frame size in bytes.
    pub fn with_max_frame_size(mut self, size: NonZeroUsize) -> Self {
        self.max_frame_size = Some(size);
        self
    }

    /// Override backoff timings using milliseconds.
    pub fn with_backoff(
        mut self,
        base_ms: Option<NonZeroU64>,
        cap_ms: Option<NonZeroU64>,
        reset_after_ms: Option<NonZeroU64>,
        deadline_ms: Option<NonZeroU64>,
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
        if self.transport.is_none() {
            return Err(HandlerBuildError::InvalidConfig(
                "socket handler requires a transport".into(),
            ));
        }
        if self.capacity.is_none() && self.capacity_set {
            return Err(HandlerBuildError::InvalidConfig(
                "capacity must be greater than zero".into(),
            ));
        }
        if let Some(timeout) = self.connect_timeout_ms {
            if timeout.get() == 0 {
                return Err(HandlerBuildError::InvalidConfig(
                    "connect_timeout_ms must be greater than zero".into(),
                ));
            }
        }
        if let Some(timeout) = self.write_timeout_ms {
            if timeout.get() == 0 {
                return Err(HandlerBuildError::InvalidConfig(
                    "write_timeout_ms must be greater than zero".into(),
                ));
            }
        }
        if let Some(size) = self.max_frame_size {
            if size.get() == 0 {
                return Err(HandlerBuildError::InvalidConfig(
                    "max_frame_size must be greater than zero".into(),
                ));
            }
        }
        Ok(())
    }

    fn build_config(&self) -> Result<SocketHandlerConfig, HandlerBuildError> {
        self.validate()?;
        let mut config = SocketHandlerConfig::default();
        if let Some(capacity) = self.capacity {
            config.capacity = capacity.get();
        }
        if let Some(timeout) = self.connect_timeout_ms {
            config.connect_timeout = Duration::from_millis(timeout.get());
        }
        if let Some(timeout) = self.write_timeout_ms {
            config.write_timeout = Duration::from_millis(timeout.get());
        }
        if let Some(size) = self.max_frame_size {
            config.max_frame_size = size.get();
        }
        if let Some(ref transport) = self.transport {
            config.transport = match transport {
                TransportConfig::Tcp { host, port, tls } => {
                    if host.trim().is_empty() {
                        return Err(HandlerBuildError::InvalidConfig(
                            "tcp host must not be empty".into(),
                        ));
                    }
                    let tls_options = tls.as_ref().map(|tls_cfg| {
                        let domain = tls_cfg
                            .domain
                            .clone()
                            .filter(|d| !d.trim().is_empty())
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
        self.backoff.apply(&mut config.backoff);
        Ok(config)
    }

    #[cfg(feature = "python")]
    fn extend_dict(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        if let Some(cap) = self.capacity {
            d.set_item("capacity", cap.get())?;
        }
        if let Some(timeout) = self.connect_timeout_ms {
            d.set_item("connect_timeout_ms", timeout.get())?;
        }
        if let Some(timeout) = self.write_timeout_ms {
            d.set_item("write_timeout_ms", timeout.get())?;
        }
        if let Some(size) = self.max_frame_size {
            d.set_item("max_frame_size", size.get())?;
        }
        match &self.transport {
            Some(TransportConfig::Tcp { host, port, tls }) => {
                d.set_item("transport", "tcp")?;
                d.set_item("host", host)?;
                d.set_item("port", *port)?;
                if let Some(tls_cfg) = tls {
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
        if let Some(base) = self.backoff.base_ms {
            d.set_item("backoff_base_ms", base.get())?;
        }
        if let Some(cap) = self.backoff.cap_ms {
            d.set_item("backoff_cap_ms", cap.get())?;
        }
        if let Some(reset) = self.backoff.reset_after_ms {
            d.set_item("backoff_reset_after_ms", reset.get())?;
        }
        if let Some(deadline) = self.backoff.deadline_ms {
            d.set_item("backoff_deadline_ms", deadline.get())?;
        }
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
    #[pyo3(signature = (host=None, port=None, *, unix_path=None))]
    fn py_new(
        host: Option<String>,
        port: Option<u16>,
        unix_path: Option<String>,
    ) -> PyResult<Self> {
        let mut builder = Self::new();
        match (host, port, unix_path) {
            (Some(h), Some(p), None) => {
                builder = builder.with_tcp(h, p);
                Ok(builder)
            }
            (None, None, Some(path)) => Ok(builder.with_unix_path(path)),
            (None, None, None) => Ok(builder),
            _ => Err(pyo3::exceptions::PyValueError::new_err(
                "provide either (host, port) or unix_path",
            )),
        }
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
        if capacity == 0 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "capacity must be greater than zero",
            ));
        }
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
        let timeout = NonZeroU64::new(timeout_ms).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err("connect_timeout_ms must be greater than zero")
        })?;
        let updated = slf.clone().with_connect_timeout_ms(timeout);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_write_timeout_ms")]
    #[pyo3(signature = (timeout_ms))]
    fn py_with_write_timeout<'py>(
        mut slf: PyRefMut<'py, Self>,
        timeout_ms: u64,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let timeout = NonZeroU64::new(timeout_ms).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err("write_timeout_ms must be greater than zero")
        })?;
        let updated = slf.clone().with_write_timeout_ms(timeout);
        *slf = updated;
        Ok(slf)
    }

    #[pyo3(name = "with_max_frame_size")]
    #[pyo3(signature = (size))]
    fn py_with_max_frame_size<'py>(
        mut slf: PyRefMut<'py, Self>,
        size: u64,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let size = NonZeroUsize::new(size as usize).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err("max_frame_size must be greater than zero")
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
        let updated = slf
            .clone()
            .with_tls_domain(domain)
            .with_insecure_tls(insecure);
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
        let base = match base_ms {
            Some(value) => Some(NonZeroU64::new(value).ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err("base_ms must be greater than zero")
            })?),
            None => None,
        };
        let cap = match cap_ms {
            Some(value) => Some(NonZeroU64::new(value).ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err("cap_ms must be greater than zero")
            })?),
            None => None,
        };
        let reset = match reset_after_ms {
            Some(value) => Some(NonZeroU64::new(value).ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err("reset_after_ms must be greater than zero")
            })?),
            None => None,
        };
        let deadline = match deadline_ms {
            Some(value) => Some(NonZeroU64::new(value).ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err("deadline_ms must be greater than zero")
            })?),
            None => None,
        };
        let updated = slf.clone().with_backoff(base, cap, reset, deadline);
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
