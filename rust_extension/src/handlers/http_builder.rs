//! Builder for [`FemtoHTTPHandler`](crate::http_handler::FemtoHTTPHandler).
//!
//! Exposes URL configuration, authentication, timeouts, serialisation format,
//! and exponential backoff parameters. The builder mirrors configuration
//! concepts described in the design documents to keep the Python and Rust
//! APIs aligned.

use std::{collections::HashMap, time::Duration};

#[cfg(feature = "python")]
use pyo3::{Bound, prelude::*, types::PyDict};

use crate::http_handler::{
    AuthConfig, FemtoHTTPHandler, HTTPHandlerConfig, HTTPMethod, SerializationFormat,
};

use super::socket_builder::BackoffOverrides;
use super::{HandlerBuildError, HandlerBuilderTrait};

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

/// Builder for constructing [`FemtoHTTPHandler`] instances.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Clone, Debug, Default)]
pub struct HTTPHandlerBuilder {
    url: Option<String>,
    method: Option<HTTPMethod>,
    auth: Option<AuthConfig>,
    headers: HashMap<String, String>,
    capacity: Option<usize>,
    connect_timeout_ms: Option<u64>,
    write_timeout_ms: Option<u64>,
    backoff: BackoffOverrides,
    format: SerializationFormat,
    record_fields: Option<Vec<String>>,
}

impl HTTPHandlerBuilder {
    /// Create a new builder with no URL configured.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the target URL for HTTP requests (required).
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the HTTP method (GET or POST). Defaults to POST.
    pub fn with_method(mut self, method: HTTPMethod) -> Self {
        self.method = Some(method);
        self
    }

    /// Configure HTTP Basic authentication.
    pub fn with_basic_auth(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.auth = Some(AuthConfig::Basic {
            username: username.into(),
            password: password.into(),
        });
        self
    }

    /// Configure Bearer token authentication.
    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.auth = Some(AuthConfig::Bearer {
            token: token.into(),
        });
        self
    }

    /// Add custom HTTP headers to requests.
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    /// Add a single custom HTTP header.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    option_setter!(
        #[doc = "Set the bounded channel capacity."]
        with_capacity,
        capacity,
        usize
    );
    option_setter!(
        #[doc = "Set the connect timeout in milliseconds."]
        with_connect_timeout_ms,
        connect_timeout_ms,
        u64
    );
    option_setter!(
        #[doc = "Set the write/request timeout in milliseconds."]
        with_write_timeout_ms,
        write_timeout_ms,
        u64
    );

    /// Override backoff timings using the provided overrides.
    pub fn with_backoff(mut self, overrides: BackoffOverrides) -> Self {
        self.backoff = overrides;
        self
    }

    /// Enable JSON serialisation format instead of URL-encoded.
    pub fn with_json_format(mut self) -> Self {
        self.format = SerializationFormat::Json;
        self
    }

    /// Limit serialised output to the specified fields.
    pub fn with_record_fields(mut self, fields: Vec<String>) -> Self {
        self.record_fields = Some(fields);
        self
    }

    fn validate(&self) -> Result<(), HandlerBuildError> {
        self.validate_url()?;
        self.validate_capacity()?;
        self.validate_timeouts()?;
        Ok(())
    }

    fn validate_url(&self) -> Result<(), HandlerBuildError> {
        match &self.url {
            None => Err(HandlerBuildError::InvalidConfig(
                "HTTP handler requires a URL".into(),
            )),
            Some(url) if url.trim().is_empty() => Err(HandlerBuildError::InvalidConfig(
                "URL must not be empty".into(),
            )),
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

    fn build_config(&self) -> Result<HTTPHandlerConfig, HandlerBuildError> {
        self.validate()?;

        let defaults = HTTPHandlerConfig::default();
        let mut config = HTTPHandlerConfig {
            url: self.url.clone().unwrap_or_default(),
            method: self.method.clone().unwrap_or(defaults.method),
            auth: self.auth.clone().unwrap_or(defaults.auth),
            headers: self.headers.clone(),
            capacity: self.capacity.unwrap_or(defaults.capacity),
            connect_timeout: self
                .connect_timeout_ms
                .map_or(defaults.connect_timeout, Duration::from_millis),
            write_timeout: self
                .write_timeout_ms
                .map_or(defaults.write_timeout, Duration::from_millis),
            format: self.format.clone(),
            record_fields: self.record_fields.clone(),
            backoff: defaults.backoff,
            warn_interval: defaults.warn_interval,
        };

        self.backoff.apply(&mut config.backoff)?;
        Ok(config)
    }

    #[cfg(feature = "python")]
    fn extend_dict(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        if let Some(ref url) = self.url {
            d.set_item("url", url)?;
        }
        if let Some(ref method) = self.method {
            d.set_item("method", method.as_str())?;
        }
        match &self.auth {
            Some(AuthConfig::Basic { username, .. }) => {
                d.set_item("auth_type", "basic")?;
                d.set_item("auth_user", username)?;
            }
            Some(AuthConfig::Bearer { .. }) => {
                d.set_item("auth_type", "bearer")?;
            }
            Some(AuthConfig::None) | None => {}
        }
        if !self.headers.is_empty() {
            let headers_dict = PyDict::new(d.py());
            for (k, v) in &self.headers {
                headers_dict.set_item(k, v)?;
            }
            d.set_item("headers", headers_dict)?;
        }
        dict_set!(d, "capacity", self.capacity);
        dict_set!(d, "connect_timeout_ms", self.connect_timeout_ms);
        dict_set!(d, "write_timeout_ms", self.write_timeout_ms);
        d.set_item(
            "format",
            match self.format {
                SerializationFormat::UrlEncoded => "url_encoded",
                SerializationFormat::Json => "json",
            },
        )?;
        if let Some(ref fields) = self.record_fields {
            d.set_item("record_fields", fields.clone())?;
        }
        Ok(())
    }
}

impl HandlerBuilderTrait for HTTPHandlerBuilder {
    type Handler = FemtoHTTPHandler;

    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError> {
        let config = self.build_config()?;
        Ok(FemtoHTTPHandler::with_config(config))
    }
}

#[cfg(feature = "python")]
mod python_bindings;
