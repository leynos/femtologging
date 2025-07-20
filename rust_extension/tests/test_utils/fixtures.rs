use super::shared_buffer::std::{Arc as StdArc, Mutex as StdMutex, SharedBuf};
use _femtologging_rs::{DefaultFormatter, FemtoStreamHandler};
use rstest::fixture;

#[fixture]
pub fn handler_tuple() -> (StdArc<StdMutex<Vec<u8>>>, FemtoStreamHandler) {
    let buffer = StdArc::new(StdMutex::new(Vec::new()));
    let handler = FemtoStreamHandler::new(SharedBuf(StdArc::clone(&buffer)), DefaultFormatter);
    (buffer, handler)
}
