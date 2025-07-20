use super::shared_buffer::std::{Arc as StdArc, Mutex as StdMutex, SharedBuf};
use _femtologging_rs::{
    rate_limited_warner::RateLimitedWarner, DefaultFormatter, FemtoStreamHandler,
};
use rstest::fixture;
use std::time::Duration;

#[fixture]
pub fn handler_tuple() -> (StdArc<StdMutex<Vec<u8>>>, FemtoStreamHandler) {
    let buffer = StdArc::new(StdMutex::new(Vec::new()));
    let handler = FemtoStreamHandler::new(SharedBuf(StdArc::clone(&buffer)), DefaultFormatter);
    (buffer, handler)
}

#[fixture]
pub fn handler_tuple_custom(
    #[default(Duration::from_secs(5))] warn_interval: Duration,
) -> (StdArc<StdMutex<Vec<u8>>>, FemtoStreamHandler) {
    let buffer = StdArc::new(StdMutex::new(Vec::new()));
    let warner = RateLimitedWarner::new(warn_interval);
    let handler = FemtoStreamHandler::with_capacity_timeout_warner(
        SharedBuf(StdArc::clone(&buffer)),
        DefaultFormatter,
        1,
        Duration::from_millis(50),
        warner,
    );
    (buffer, handler)
}
