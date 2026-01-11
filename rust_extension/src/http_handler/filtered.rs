//! Filtered serialization for HTTP payloads.
//!
//! Provides field-filtered serialization that selectively includes
//! only the requested fields from a log record.

use std::collections::HashSet;

use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};

use super::record::HttpSerializableRecord;

/// Wrapper for filtered serialization with zero-copy where possible.
pub(super) struct FilteredRecord<'a> {
    pub(super) record: HttpSerializableRecord<'a>,
    pub(super) fields: &'a HashSet<&'a str>,
}

/// Base field names for counting and iteration.
const BASE_FIELDS: [&str; 8] = [
    "name",
    "levelname",
    "msg",
    "created",
    "filename",
    "lineno",
    "module",
    "thread",
];

impl<'a> FilteredRecord<'a> {
    /// Count the total number of fields that will be serialized.
    fn count_fields(&self) -> usize {
        let r = &self.record;
        let f = self.fields;

        let base_count = BASE_FIELDS.iter().filter(|&&k| f.contains(k)).count();
        let optional_count = usize::from(f.contains("threadName") && r.thread_name.is_some())
            + usize::from(f.contains("exc_info") && r.exc_info.is_some())
            + usize::from(f.contains("stack_info") && r.stack_info.is_some())
            + r.key_values
                .keys()
                .filter(|k| f.contains(k.as_str()))
                .count();

        base_count + optional_count
    }

    /// Serialize key-value pairs that match the filter.
    fn serialize_key_values<S>(
        &self,
        map: &mut <S as Serializer>::SerializeMap,
    ) -> Result<(), S::Error>
    where
        S: Serializer,
    {
        for (k, v) in self.record.key_values {
            if self.fields.contains(k.as_str()) {
                map.serialize_entry(k, v)?;
            }
        }
        Ok(())
    }

    /// Serialize optional fields (threadName, key_values, exc_info, stack_info).
    fn serialize_optional_fields<S>(
        &self,
        map: &mut <S as Serializer>::SerializeMap,
    ) -> Result<(), S::Error>
    where
        S: Serializer,
    {
        let r = &self.record;
        let f = self.fields;

        if f.contains("threadName")
            && let Some(name) = r.thread_name
        {
            map.serialize_entry("threadName", name)?;
        }
        self.serialize_key_values::<S>(map)?;
        if f.contains("exc_info")
            && let Some(exc) = r.exc_info
        {
            map.serialize_entry("exc_info", exc)?;
        }
        if f.contains("stack_info")
            && let Some(stack) = r.stack_info
        {
            map.serialize_entry("stack_info", stack)?;
        }
        Ok(())
    }
}

impl Serialize for FilteredRecord<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let r = &self.record;
        let f = self.fields;

        let mut map = serializer.serialize_map(Some(self.count_fields()))?;

        macro_rules! emit {
            ($key:literal, $val:expr) => {
                if f.contains($key) {
                    map.serialize_entry($key, $val)?;
                }
            };
        }

        emit!("name", r.name);
        emit!("levelname", r.levelname);
        emit!("msg", r.msg);
        emit!("created", &r.created);
        emit!("filename", r.filename);
        emit!("lineno", &r.lineno);
        emit!("module", r.module);
        if f.contains("thread") {
            map.serialize_entry("thread", &r.thread_string())?;
        }

        self.serialize_optional_fields::<S>(&mut map)?;
        map.end()
    }
}
