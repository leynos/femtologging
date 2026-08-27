//! Python dictionary serialisation for the socket handler builder.

use pyo3::{
    Bound, PyResult,
    types::{PyDict, PyDictMethods},
};

use crate::handlers::builder_macros::dict_set;

use super::super::{SocketHandlerBuilder, TransportConfig};

impl SocketHandlerBuilder {
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
            Some(TransportConfig::Tcp { host, port }) => {
                d.set_item("transport", "tcp")?;
                d.set_item("host", host)?;
                d.set_item("port", *port)?;
                self.extend_dict_with_tls(d)
            }
            Some(TransportConfig::Unix { path }) => {
                d.set_item("transport", "unix")?;
                d.set_item("path", path.display().to_string())
            }
            None => Ok(()),
        }
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
