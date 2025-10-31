//! MessagePack serialisation helpers.

use std::io;

use rmp_serde::Serializer;
use serde::Serialize;

use crate::log_record::FemtoLogRecord;

#[derive(Serialize)]
struct SerializableRecord<'a> {
    logger: &'a str,
    level: &'a str,
    message: &'a str,
    timestamp_ns: u128,
    filename: &'a str,
    line_number: u32,
    module_path: &'a str,
    thread_id: String,
    thread_name: Option<&'a str>,
    key_values: &'a std::collections::BTreeMap<String, String>,
}

impl<'a> From<&'a FemtoLogRecord> for SerializableRecord<'a> {
    fn from(record: &'a FemtoLogRecord) -> Self {
        let timestamp_ns = record
            .metadata
            .timestamp
            .duration_since(std::time::UNIX_EPOCH)
            .map(|dur| dur.as_nanos())
            .unwrap_or_default();

        Self {
            logger: &record.logger,
            level: &record.level,
            message: &record.message,
            timestamp_ns,
            filename: &record.metadata.filename,
            line_number: record.metadata.line_number,
            module_path: &record.metadata.module_path,
            thread_id: format!("{:?}", record.metadata.thread_id),
            thread_name: record.metadata.thread_name.as_deref(),
            key_values: &record.metadata.key_values,
        }
    }
}

/// Serialise a record into a MessagePack payload.
pub fn serialise_record(record: &FemtoLogRecord) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(128);
    let serialisable = SerializableRecord::from(record);
    serialisable
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
