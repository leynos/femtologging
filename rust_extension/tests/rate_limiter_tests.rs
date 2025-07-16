use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use _femtologging_rs::rate_limiter::RateLimiter;
use logtest::Logger;

fn mock_time_provider(time: Arc<AtomicU64>) -> impl Fn() -> u64 {
    move || time.load(Ordering::Relaxed)
}

#[test]
fn rate_limiter_issues_warnings() {
    let mut logger = Logger::start();
    let time = Arc::new(AtomicU64::new(0));
    let limiter = RateLimiter::new(
        "TestHandler",
        5,
        Box::new(mock_time_provider(time.clone())),
    );

    // First dropped record should not trigger a warning
    limiter.record_dropped();
    assert!(logger.pop().is_none());

    // Advance time, but not enough to trigger a warning
    time.store(4, Ordering::Relaxed);
    limiter.record_dropped();
    assert!(logger.pop().is_none());

    // Advance time enough to trigger a warning
    time.store(5, Ordering::Relaxed);
    limiter.record_dropped();
    let log = logger.pop().expect("no log produced");
    assert_eq!(log.level(), log::Level::Warn);
    assert!(log.args().contains("3 log records dropped"));

    // Subsequent dropped records should not trigger a warning
    limiter.record_dropped();
    assert!(logger.pop().is_none());
}
