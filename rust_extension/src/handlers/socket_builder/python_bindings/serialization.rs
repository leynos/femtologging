//! Python dictionary serialization for the socket handler builder.
//!
//! `SocketHandlerBuilder::extend_dict` renders the builder's capacity,
//! timeouts, backoff and filter settings into the dict returned by the
//! `as_dict` method and the `AsPyDict` implementation. Transport helpers do
//! the same for each endpoint: `extend_dict_with_tcp` writes the host, port
//! and TLS options, `extend_dict_with_unix` writes the socket path, and both
//! are reached through `extend_dict_with_transport`, so a builder with no
//! transport configured adds no transport keys at all.

use std::path::Path;

use pyo3::{
    Bound, PyResult,
    types::{PyDict, PyDictMethods},
};

use crate::handlers::builder_macros::dict_set;

use super::super::{SocketHandlerBuilder, TransportConfig};

impl SocketHandlerBuilder {
    /// Serialize configured transport, retry, and filter options for Python inspection.
    pub(super) fn extend_dict(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        dict_set!(d, "capacity", self.capacity);
        dict_set!(d, "connect_timeout_ms", self.connect_timeout_ms);
        dict_set!(d, "write_timeout_ms", self.write_timeout_ms);
        dict_set!(d, "max_frame_size", self.max_frame_size);
        self.extend_dict_with_transport(d)?;
        dict_set!(d, "backoff_base_ms", self.backoff.base_ms);
        dict_set!(d, "backoff_cap_ms", self.backoff.cap_ms);
        dict_set!(d, "backoff_reset_after_ms", self.backoff.reset_after_ms);
        dict_set!(d, "backoff_deadline_ms", self.backoff.deadline_ms);
        if !self.filters.is_empty() {
            d.set_item("filters", self.filters.clone())?;
        }
        Ok(())
    }

    /// Serialize the configured transport and any TLS options into the dict.
    fn extend_dict_with_transport(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        match &self.transport {
            Some(TransportConfig::Tcp { host, port }) => self.extend_dict_with_tcp(d, host, *port),
            Some(TransportConfig::Unix { path }) => extend_dict_with_unix(d, path),
            None => Ok(()),
        }
    }

    /// Serialize a TCP endpoint, including the TLS options that accompany it.
    fn extend_dict_with_tcp(&self, d: &Bound<'_, PyDict>, host: &str, port: u16) -> PyResult<()> {
        d.set_item("transport", "tcp")?;
        d.set_item("host", host)?;
        d.set_item("port", port)?;
        self.extend_dict_with_tls(d)
    }

    /// Serialize the TLS options for a TCP transport into the dict.
    fn extend_dict_with_tls(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        let Some(tls_cfg) = &self.tls else {
            return d.set_item("tls", false);
        };
        d.set_item("tls", true)?;
        if let Some(domain) = &tls_cfg.domain {
            d.set_item("tls_domain", domain)?;
        }
        d.set_item("tls_insecure", tls_cfg.insecure)
    }
}

/// Serialize a Unix-domain socket endpoint.
fn extend_dict_with_unix(d: &Bound<'_, PyDict>, path: &Path) -> PyResult<()> {
    d.set_item("transport", "unix")?;
    d.set_item("path", path.display().to_string())
}
