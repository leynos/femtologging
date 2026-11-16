//! Python dictionary serialization for the socket handler builder.
//!
//! Each transport writes its own keys: the Unix variant records its path, and
//! the TCP variant records its host and port before any TLS options, so a
//! reader can tell which fields belong to which transport.

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
        self.extend_dict_with_scalars(d)?;
        self.extend_dict_with_transport(d)?;
        self.extend_dict_with_backoff(d)?;
        self.extend_dict_with_filters(d)
    }

    /// Serialize the optional scalar builder fields into the dict.
    fn extend_dict_with_scalars(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        dict_set!(d, "capacity", self.capacity);
        dict_set!(d, "connect_timeout_ms", self.connect_timeout_ms);
        dict_set!(d, "write_timeout_ms", self.write_timeout_ms);
        dict_set!(d, "max_frame_size", self.max_frame_size);
        Ok(())
    }

    /// Serialize the configured transport and any TLS options into the dict.
    fn extend_dict_with_transport(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        match &self.transport {
            Some(TransportConfig::Tcp { host, port }) => {
                self.extend_dict_with_tcp_transport(d, host, *port)
            }
            Some(TransportConfig::Unix { path }) => Self::extend_dict_with_unix_transport(d, path),
            None => Ok(()),
        }
    }

    /// Serialize a TCP transport, whose TLS options are configured separately.
    fn extend_dict_with_tcp_transport(
        &self,
        d: &Bound<'_, PyDict>,
        host: &str,
        port: u16,
    ) -> PyResult<()> {
        d.set_item("transport", "tcp")?;
        d.set_item("host", host)?;
        d.set_item("port", port)?;
        self.extend_dict_with_tls(d)
    }

    /// Serialize a Unix-domain transport, whose path can be displayed lossily.
    fn extend_dict_with_unix_transport(d: &Bound<'_, PyDict>, path: &Path) -> PyResult<()> {
        d.set_item("transport", "unix")?;
        d.set_item("path", path.display().to_string())
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

    /// Serialize the configured backoff overrides into the dict.
    fn extend_dict_with_backoff(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        dict_set!(d, "backoff_base_ms", self.backoff.base_delay);
        dict_set!(d, "backoff_cap_ms", self.backoff.max_delay);
        dict_set!(d, "backoff_reset_after_ms", self.backoff.reset_after);
        dict_set!(d, "backoff_deadline_ms", self.backoff.deadline);
        Ok(())
    }

    /// Serialize the configured filter identifiers into the dict.
    fn extend_dict_with_filters(&self, d: &Bound<'_, PyDict>) -> PyResult<()> {
        if !self.filters.is_empty() {
            d.set_item("filters", self.filters.clone())?;
        }
        Ok(())
    }
}
