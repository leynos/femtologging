//! Builder for [`FemtoSocketHandler`](crate::socket_handler::FemtoSocketHandler).
//!
//! Exposes transport selection, timeout tuning, TLS configuration, and
//! exponential backoff parameters. The builder mirrors configuration concepts
//! described in the design documents to keep the Python and Rust APIs aligned.

use std::{path::PathBuf, time::Duration};

#[cfg(feature = "python")]
use pyo3::{prelude::*, types::PyDict, Bound};

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

/// Overrides for the socket backoff timings applied by the handler.
#[derive(Clone, Debug, Default)]
pub struct BackoffOverrides {
    base_ms: Option<u64>,
    cap_ms: Option<u64>,
    reset_after_ms: Option<u64>,
    deadline_ms: Option<u64>,
}

macro_rules! ensure_positive {
    ($value:expr, $field:expr) => {{
        if $value == 0 {
            Err(HandlerBuildError::InvalidConfig(format!(
                "{} must be greater than zero",
                $field
            )))
        } else {
            Ok($value)
        }
    }};
}

impl BackoffOverrides {
    /// Create overrides with no custom values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the base jitter duration in milliseconds.
    pub fn with_base_ms(mut self, base_ms: u64) -> Self {
        self.base_ms = Some(base_ms);
        self
    }

    /// Override the cap duration in milliseconds.
    pub fn with_cap_ms(mut self, cap_ms: u64) -> Self {
        self.cap_ms = Some(cap_ms);
        self
    }

    /// Override the reset-after duration in milliseconds.
    pub fn with_reset_after_ms(mut self, reset_after_ms: u64) -> Self {
        self.reset_after_ms = Some(reset_after_ms);
        self
    }

    /// Override the deadline duration in milliseconds.
    pub fn with_deadline_ms(mut self, deadline_ms: u64) -> Self {
        self.deadline_ms = Some(deadline_ms);
        self
    }

    #[cfg(feature = "python")]
    pub(crate) fn from_options(
        base_ms: Option<u64>,
        cap_ms: Option<u64>,
        reset_after_ms: Option<u64>,
        deadline_ms: Option<u64>,
    ) -> Self {
        Self {
            base_ms,
            cap_ms,
            reset_after_ms,
            deadline_ms,
        }
    }

    fn apply(&self, policy: &mut BackoffPolicy) -> Result<(), HandlerBuildError> {
        if let Some(base) = self.base_ms {
            ensure_positive!(base, "backoff_base_ms")?;
            policy.base = Duration::from_millis(base);
        }
        if let Some(cap) = self.cap_ms {
            ensure_positive!(cap, "backoff_cap_ms")?;
            policy.cap = Duration::from_millis(cap);
        }
        if let Some(reset) = self.reset_after_ms {
            ensure_positive!(reset, "backoff_reset_after_ms")?;
            policy.reset_after = Duration::from_millis(reset);
        }
        if let Some(deadline) = self.deadline_ms {
            ensure_positive!(deadline, "backoff_deadline_ms")?;
            policy.deadline = Duration::from_millis(deadline);
        }
        Ok(())
    }
}

macro_rules! option_setter {
    ($(#[$meta:meta])* $fn_name:ident, $field:ident, $ty:ty) => {
        $(#[$meta])*
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
    backoff: BackoffOverrides,
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

    option_setter!(
        #[doc = "Set the bounded channel capacity."]
        with_capacity,
        capacity,
        usize
    );
    option_setter!(with_connect_timeout_ms, connect_timeout_ms, u64);
    option_setter!(with_write_timeout_ms, write_timeout_ms, u64);
    option_setter!(with_max_frame_size, max_frame_size, usize);

    /// Override backoff timings using the provided overrides.
    ///
    /// See [`BackoffOverrides`] for fluent helpers when constructing the
    /// override set from Rust.
    pub fn with_backoff(mut self, overrides: BackoffOverrides) -> Self {
        self.backoff = overrides;
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
            ensure_positive!(capacity, "capacity")?;
        }
        Ok(())
    }

    fn validate_timeouts(&self) -> Result<(), HandlerBuildError> {
        if let Some(timeout) = self.connect_timeout_ms {
            ensure_positive!(timeout, "connect_timeout_ms")?;
        }
        if let Some(timeout) = self.write_timeout_ms {
            ensure_positive!(timeout, "write_timeout_ms")?;
        }
        Ok(())
    }

    fn validate_frame_size(&self) -> Result<(), HandlerBuildError> {
        if let Some(size) = self.max_frame_size {
            ensure_positive!(size, "max_frame_size")?;
        }
        Ok(())
    }

    fn build_config(&self) -> Result<SocketHandlerConfig, HandlerBuildError> {
        self.validate()?;
        let mut config = SocketHandlerConfig::default();
        self.apply_optional_fields(&mut config);
        if let Some(ref transport) = self.transport {
            config.transport = self.build_transport_config(transport)?;
        }
        self.backoff.apply(&mut config.backoff)?;
        Ok(config)
    }

    fn apply_optional_fields(&self, config: &mut SocketHandlerConfig) {
        if let Some(capacity) = self.capacity {
            config.capacity = capacity;
        }
        if let Some(timeout) = self.connect_timeout_ms {
            config.connect_timeout = Duration::from_millis(timeout);
        }
        if let Some(timeout) = self.write_timeout_ms {
            config.write_timeout = Duration::from_millis(timeout);
        }
        if let Some(size) = self.max_frame_size {
            config.max_frame_size = size;
        }
    }

    fn build_transport_config(
        &self,
        transport: &TransportConfig,
    ) -> Result<SocketTransport, HandlerBuildError> {
        match transport {
            TransportConfig::Tcp { host, port } => {
                if host.trim().is_empty() {
                    return Err(HandlerBuildError::InvalidConfig(
                        "tcp host must not be empty".into(),
                    ));
                }
                let tls_options = self.build_tls_options(host);
                Ok(SocketTransport::Tcp(TcpTransport {
                    host: host.clone(),
                    port: *port,
                    tls: tls_options,
                }))
            }
            TransportConfig::Unix { path } => {
                Ok(SocketTransport::Unix(UnixTransport { path: path.clone() }))
            }
        }
    }

    fn build_tls_options(&self, host: &str) -> Option<TlsOptions> {
        self.tls.as_ref().map(|tls_cfg| {
            let domain = tls_cfg
                .domain
                .clone()
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| host.to_owned());
            TlsOptions {
                domain,
                insecure_skip_verify: tls_cfg.insecure,
            }
        })
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
mod python_bindings;
