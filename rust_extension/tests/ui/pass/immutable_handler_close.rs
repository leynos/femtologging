//! Compile-pass coverage for closing handlers through shared references.
//!
//! Every handler close takes `&self` and may be called concurrently, so a
//! binding that is never declared `mut` must still close, and closing twice
//! must remain type-correct because close is idempotent.

use _femtologging_rs::{DefaultFormatter, FemtoFileHandler, FemtoStreamHandler};

fn main() {
    // The helper is never called: this fixture proves the calls type-check
    // without touching the filesystem.
    #[allow(dead_code)]
    fn close_through_shared_references() {
        let file = FemtoFileHandler::new("immutable-close.log")
            .expect("the probe file should open for writing");
        let stream = FemtoStreamHandler::new(std::io::sink(), DefaultFormatter);

        let file_ref: &FemtoFileHandler = &file;
        let stream_ref: &FemtoStreamHandler = &stream;

        file_ref.close();
        stream_ref.close();

        // Repeated and concurrent closes are type-correct on the same binding.
        file.close();
        stream.close();
    }
}
