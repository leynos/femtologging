//! Compile-pass coverage for file-handler test support.

#[path = "../../../src/handlers/file/test_support.rs"]
mod test_support;

use std::io::{self, Seek, SeekFrom, Write};

#[derive(Default)]
struct TestWriter;

impl Write for TestWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

test_support::impl_unsupported_seek!(TestWriter);

fn main() {
    let mut writer = TestWriter;
    let error = writer
        .seek(SeekFrom::Start(0))
        .expect_err("the test writer should reject seeking");
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
}
