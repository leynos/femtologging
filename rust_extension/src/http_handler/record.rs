//! Zero-copy serializable record for HTTP payloads.
//!
//! Provides an intermediate representation that borrows from the original
//! log record to avoid allocations during serialization.

use std::collections::BTreeMap;
use std::thread::ThreadId;

use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};

use crate::exception_schema::{ExceptionPayload, StackTracePayload};
use crate::log_record::FemtoLogRecord;

/// Zero-copy serializable record for HTTP payloads.
///
/// This struct borrows from the original record to avoid allocations for string
/// fields during serialization. The `level` field holds a `&'static str` from
/// `FemtoLevel::as_str()`, enabling zero-allocation level serialization.
/// The `thread_id` field stores the raw `ThreadId` and is formatted only during
/// serialization, avoiding per-record String allocations.
pub(super) struct HttpSerializableRecord<'a> {
    pub(super) name: &'a str,
    pub(super) levelname: &'static str,
    pub(super) msg: &'a str,
    pub(super) created: f64,
    pub(super) filename: &'a str,
    pub(super) lineno: u32,
    pub(super) module: &'a str,
    pub(super) thread_id: ThreadId,
    pub(super) thread_name: Option<&'a str>,
    pub(super) key_values: &'a BTreeMap<String, String>,
    pub(super) exc_info: Option<&'a ExceptionPayload>,
    pub(super) stack_info: Option<&'a StackTracePayload>,
}

impl HttpSerializableRecord<'_> {
    /// Count the total number of fields that will be serialized.
    fn count_fields(&self) -> usize {
        8 + usize::from(self.thread_name.is_some())
            + self.key_values.len()
            + usize::from(self.exc_info.is_some())
            + usize::from(self.stack_info.is_some())
    }
}

impl<'a> From<&'a FemtoLogRecord> for HttpSerializableRecord<'a> {
    fn from(record: &'a FemtoLogRecord) -> Self {
        let metadata = record.metadata();
        let created = metadata
            .timestamp
            .duration_since(std::time::UNIX_EPOCH)
            .map(|dur| dur.as_secs_f64())
            .unwrap_or_default();

        Self {
            name: record.logger(),
            levelname: record.level_str(),
            msg: record.message(),
            created,
            filename: &metadata.filename,
            lineno: metadata.line_number,
            module: &metadata.module_path,
            thread_id: metadata.thread_id,
            thread_name: metadata.thread_name.as_deref(),
            key_values: &metadata.key_values,
            exc_info: record.exception_payload(),
            stack_info: record.stack_payload(),
        }
    }
}

impl Serialize for HttpSerializableRecord<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.count_fields()))?;
        map.serialize_entry("name", self.name)?;
        map.serialize_entry("levelname", self.levelname)?;
        map.serialize_entry("msg", self.msg)?;
        map.serialize_entry("created", &self.created)?;
        map.serialize_entry("filename", self.filename)?;
        map.serialize_entry("lineno", &self.lineno)?;
        map.serialize_entry("module", self.module)?;
        map.serialize_entry("thread", &format_args!("{:?}", self.thread_id))?;
        if let Some(name) = self.thread_name {
            map.serialize_entry("threadName", name)?;
        }
        for (k, v) in self.key_values {
            map.serialize_entry(k, v)?;
        }
        if let Some(exc) = self.exc_info {
            map.serialize_entry("exc_info", exc)?;
        }
        if let Some(stack) = self.stack_info {
            map.serialize_entry("stack_info", stack)?;
        }
        map.end()
    }
}
