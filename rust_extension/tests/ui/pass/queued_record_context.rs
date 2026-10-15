//! Compile-pass coverage for queued Python context compatibility.
//!
//! Custom handlers may rely on `FemtoHandlerTrait`'s context-aware default,
//! while `QueuedRecord` remains constructible in Python and non-Python builds.

use std::any::Any;
use std::sync::Arc;

use _femtologging_rs::{
    FemtoHandlerTrait, FemtoLevel, FemtoLogRecord, HandlerError, QueuedRecord,
};

struct NativeHandler;

impl FemtoHandlerTrait for NativeHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn main() {
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(NativeHandler);
    let record = FemtoLogRecord::new("ui", FemtoLevel::Info, "queued");

    #[cfg(feature = "python")]
    handler
        .handle_with_context(record.clone(), None)
        .expect("native handlers should use the context-aware default");

    let _job = QueuedRecord {
        record,
        handlers: vec![handler],
        #[cfg(feature = "python")]
        context: None,
    };
}
