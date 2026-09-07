//! A simple handler that accumulates records in memory for test assertions.
//!
//! This module is shared across multiple test files so that each test module
//! does not need its own copy of the same boilerplate.

use std::{any::Any, sync::Arc};

use parking_lot::Mutex;

use crate::{
    handler::{FemtoHandlerTrait, HandlerError},
    log_record::FemtoLogRecord,
};

/// Handler that stores every record it receives for later inspection.
#[derive(Clone, Default)]
pub struct CollectingHandler {
    records: Arc<Mutex<Vec<FemtoLogRecord>>>,
}

impl CollectingHandler {
    /// Create a new empty handler.
    pub fn new() -> Self { Self::default() }

    /// Return a snapshot of all records received so far.
    pub fn collected(&self) -> Vec<FemtoLogRecord> { self.records.lock().clone() }
}

impl FemtoHandlerTrait for CollectingHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.records.lock().push(record);
        Ok(())
    }

    fn as_any(&self) -> &dyn Any { self }
}
