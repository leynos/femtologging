//! Builder for [`FemtoHTTPHandler`](crate::http_handler::FemtoHTTPHandler).
//!
//! Exposes URL configuration, authentication, timeouts, serialization format,
//! and exponential backoff parameters. The builder mirrors configuration
//! concepts described in the design documents to keep the Python and Rust
//! APIs aligned.

use std::{collections::HashMap, time::Duration};

#[cfg(feature = "python")]
use pyo3::{Bound, prelude::*, types::PyDict};

use crate::http_handler::{
    AuthConfig, FemtoHTTPHandler, HTTPHandlerConfig, HTTPMethod, SerializationFormat,
};

#[cfg(feature = "python")]
use super::builder_macros::dict_set;
use super::builder_macros::ensure_positive;
use super::socket_builder::BackoffOverrides;
use super::{HandlerBuildError, HandlerBuilderTrait};

/// Generates fluent setters that store optional builder values for later validation.
macro_rules! option_setter {
    ($(#[$meta:meta])* $fn_name:ident, $field:ident, $ty:ty) => {
        $(#[$meta])*
        pub fn $fn_name(mut self, value: $ty) -> Self {
            self.$field = Some(value);
            self
        }
    };
}

/// Builder for constructing [`FemtoHTTPHandler`] instances.
#[cfg_attr(feature = "python", pyclass(from_py_object))]
#[derive(Clone, Debug, Default)]
pub struct HTTPHandlerBuilder {
    /// Required destination URL, validated before a worker is spawned.
    url: Option<String>,
    /// HTTP method, defaulting to POST when omitted.
    method: Option<HTTPMethod>,
    /// Optional Basic or Bearer credentials copied into the worker configuration.
    auth: Option<AuthConfig>,
    /// Headers copied into each worker request; duplicate keys are replaced.
    headers: HashMap<String, String>,
    /// Bounded worker queue capacity, validated as non-zero when provided.
    capacity: Option<usize>,
    /// Optional TCP connection timeout in milliseconds; zero is rejected.
    connect_timeout_ms: Option<u64>,
    /// Optional request-write timeout in milliseconds; zero is rejected.
    write_timeout_ms: Option<u64>,
    /// Per-worker retry timing overrides, each validated before construction.
    backoff: BackoffOverrides,
    /// Request encoding, URL-encoded by default or JSON when selected.
    format: SerializationFormat,
    /// Optional allow-list of record fields sent in each request.
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

    /// Replace all custom HTTP headers with the provided map.
    ///
    /// This replaces any previously configured headers. Use [`with_header`] to
    /// add individual headers without replacing existing ones.
    ///
    /// [`with_header`]: HTTPHandlerBuilder::with_header
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

    /// Enable JSON serialization format instead of URL-encoded.
    pub fn with_json_format(mut self) -> Self {
        self.format = SerializationFormat::Json;
        self
    }

    /// Limit serialized output to the specified fields.
    pub fn with_record_fields(mut self, fields: Vec<String>) -> Self {
        self.record_fields = Some(fields);
        self
    }

    /// Checks all builder values before creating worker-owned HTTP state.
    fn validate(&self) -> Result<(), HandlerBuildError> {
        self.validate_url()?;
        self.validate_capacity()?;
        self.validate_timeouts()?;
        self.validate_method_format_combination()?;
        Ok(())
    }

    /// Rejects JSON GET requests because GET payloads are appended as query data.
    fn validate_method_format_combination(&self) -> Result<(), HandlerBuildError> {
        let method = self.method.clone().unwrap_or_default();
        if method == HTTPMethod::GET && matches!(self.format, SerializationFormat::Json) {
            return Err(HandlerBuildError::InvalidConfig(
                "JSON payloads with GET are not supported; use POST or URL-encoded format".into(),
            ));
        }
        Ok(())
    }

    /// Ensures a non-empty URL is available before the worker starts.
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

    /// Rejects a zero queue capacity, which would prevent channel construction.
    fn validate_capacity(&self) -> Result<(), HandlerBuildError> {
        if let Some(capacity) = self.capacity {
            ensure_positive!(capacity, "capacity")?;
        }
        Ok(())
    }

    /// Rejects zero-valued connection and write timeouts.
    fn validate_timeouts(&self) -> Result<(), HandlerBuildError> {
        if let Some(timeout) = self.connect_timeout_ms {
            ensure_positive!(timeout, "connect_timeout_ms")?;
        }
        if let Some(timeout) = self.write_timeout_ms {
            ensure_positive!(timeout, "write_timeout_ms")?;
        }
        Ok(())
    }

    /// Resolves defaults, validates overrides, and returns the worker configuration.
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

    /// Exposes explicitly configured Python builder values as a dictionary.
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
        dict_set!(d, "backoff_base_ms", self.backoff.base_ms());
        dict_set!(d, "backoff_cap_ms", self.backoff.cap_ms());
        dict_set!(d, "backoff_reset_after_ms", self.backoff.reset_after_ms());
        dict_set!(d, "backoff_deadline_ms", self.backoff.deadline_ms());
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

#[cfg(test)]
mod tests {
    //! Tests for the HTTP handler builder.

    use super::*;

    #[test]
    fn rejects_get_with_json_format() {
        let result = HTTPHandlerBuilder::new()
            .with_url("http://example.com/log")
            .with_method(HTTPMethod::GET)
            .with_json_format()
            .build_inner();

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, HandlerBuildError::InvalidConfig(_)));
        if let HandlerBuildError::InvalidConfig(msg) = err {
            assert!(msg.contains("JSON"));
            assert!(msg.contains("GET"));
        }
    }

    #[test]
    fn allows_post_with_json_format() {
        let result = HTTPHandlerBuilder::new()
            .with_url("http://example.com/log")
            .with_method(HTTPMethod::POST)
            .with_json_format()
            .build_inner();

        assert!(result.is_ok());
    }

    #[test]
    fn allows_get_with_url_encoded_format() {
        let result = HTTPHandlerBuilder::new()
            .with_url("http://example.com/log")
            .with_method(HTTPMethod::GET)
            .build_inner();

        assert!(result.is_ok());
    }
}
