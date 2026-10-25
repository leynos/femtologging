//! Unit tests for socket frame serialization and retry backoff.

use std::time::{Duration, Instant};

use rstest::rstest;

use super::*;

#[rstest]
fn frame_payload_enforces_limit() {
    let payload = vec![0u8; 32];
    let framed = frame_payload(&payload, 16);
    assert!(
        framed.is_none(),
        "payload larger than limit must be rejected",
    );
}

#[rstest]
fn frame_payload_prefixes_length() {
    let payload = vec![1u8, 2, 3];
    let framed = frame_payload(&payload, 16).expect("payload fits frame");
    assert_eq!(&framed[..4], &3u32.to_be_bytes());
    assert_eq!(&framed[4..], payload);
}

#[rstest]
fn serialize_record_round_trips() {
    let record = FemtoLogRecord::new("logger", FemtoLevel::Info, "hello");
    let payload = serialize_record(&record).expect("serialize record");
    let decoded: Payload = rmp_serde::from_slice(&payload).expect("decode payload");
    assert_eq!(decoded.logger, "logger");
    assert_eq!(decoded.level, "INFO");
    assert_eq!(decoded.message, "hello");
}

#[rstest]
fn backoff_enforces_minimum_sleep() {
    let policy = BackoffPolicy {
        base: Duration::from_millis(0),
        cap: Duration::from_millis(0),
        deadline: Duration::from_millis(50),
        ..BackoffPolicy::default()
    };
    let mut backoff = BackoffState::new(policy);
    let now = Instant::now();
    let sleep = backoff
        .next_sleep(now)
        .expect("first backoff value must exist");
    assert!(
        sleep >= Duration::from_millis(10),
        "sleep {:?} should respect minimum",
        sleep
    );
}

#[rstest]
fn backoff_respects_deadline() {
    let policy = BackoffPolicy {
        base: Duration::from_millis(10),
        cap: Duration::from_millis(10),
        deadline: Duration::from_millis(20),
        ..BackoffPolicy::default()
    };
    let mut backoff = BackoffState::new(policy);
    let now = Instant::now();
    assert!(backoff.next_sleep(now).is_some());
    let expired = now + Duration::from_millis(25);
    assert!(backoff.next_sleep(expired).is_none());
}
