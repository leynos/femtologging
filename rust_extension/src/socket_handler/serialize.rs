//! MessagePack serialization helpers.

use std::io;

use rmp_serde::Serializer;
use serde::Serialize;

use crate::log_record::FemtoLogRecord;

/// MessagePack record shape sent after the socket frame length prefix.
#[derive(Serialize)]
struct SerializableRecord<'a> {
    /// Logger name encoded in the socket record.
    logger: &'a str,
    /// Level name encoded in the socket record.
    level: &'a str,
    /// Message text encoded in the socket record.
    message: &'a str,
    /// Record timestamp in nanoseconds since the UNIX epoch.
    timestamp_ns: u128,
    /// Source filename carried in the socket payload.
    filename: &'a str,
    /// Source line number carried in the socket payload.
    line_number: u32,
    /// Source module path carried in the socket payload.
    module_path: &'a str,
    /// Thread identifier converted to an owned string before encoding.
    thread_id: String,
    /// Optional thread name, omitted when the source record has no name.
    thread_name: Option<&'a str>,
    /// Structured record metadata encoded as key-value entries.
    key_values: &'a std::collections::BTreeMap<String, String>,
}

impl<'a> From<&'a FemtoLogRecord> for SerializableRecord<'a> {
    fn from(record: &'a FemtoLogRecord) -> Self {
        let metadata = record.metadata();
        let timestamp_ns = metadata
            .timestamp
            .duration_since(std::time::UNIX_EPOCH)
            .map(|dur| dur.as_nanos())
            .unwrap_or_default();

        Self {
            logger: record.logger(),
            level: record.level_str(),
            message: record.message(),
            timestamp_ns,
            filename: &metadata.filename,
            line_number: metadata.line_number,
            module_path: &metadata.module_path,
            thread_id: format!("{:?}", metadata.thread_id),
            thread_name: metadata.thread_name.as_deref(),
            key_values: &metadata.key_values,
        }
    }
}

/// Serialize a record into a MessagePack payload.
pub fn serialize_record(record: &FemtoLogRecord) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(128);
    let serializable = SerializableRecord::from(record);
    serializable
        .serialize(&mut Serializer::new(&mut buf).with_struct_map())
        .map_err(io::Error::other)?;
    Ok(buf)
}

/// Frame the payload with a big-endian length prefix.
pub fn frame_payload(payload: &[u8], max_size: usize) -> Option<Vec<u8>> {
    if payload.len() > max_size {
        return None;
    }
    let len = u32::try_from(payload.len()).ok()?;
    let capacity = payload.len().checked_add(4)?;
    let mut framed = Vec::with_capacity(capacity);
    framed.extend(len.to_be_bytes());
    framed.extend_from_slice(payload);
    Some(framed)
}
