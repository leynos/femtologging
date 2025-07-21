use super::shared_buffer::std::{Arc as StdArc, Mutex as StdMutex, SharedBuf};
use _femtologging_rs::{
    rate_limited_warner::RateLimitedWarner, DefaultFormatter, FemtoStreamHandler,
    StreamHandlerConfig,
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
    let handler = FemtoStreamHandler::with_test_config(
        SharedBuf(StdArc::clone(&buffer)),
        DefaultFormatter,
        StreamHandlerConfig::default()
            .with_capacity(1)
            .with_timeout(Duration::from_millis(50))
            .with_warner(RateLimitedWarner::new(warn_interval)),
    );
    (buffer, handler)
}
