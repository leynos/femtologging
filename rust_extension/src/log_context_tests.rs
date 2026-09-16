//! Unit tests for scoped context propagation helpers.

use super::*;
use rstest::{fixture, rstest};
use static_assertions::assert_not_impl_any;

#[fixture]
fn isolated_context() {
    clear_log_context_for_test();
}

#[rstest]
fn context_push_pop_round_trip(_isolated_context: ()) {
    push_log_context_map(BTreeMap::from([("request_id".into(), "123".into())]))
        .expect("context push should succeed");
    let merged = merge_context_values(&BTreeMap::new()).expect("merge should succeed");
    assert_eq!(merged.get("request_id").map(String::as_str), Some("123"));
    pop_log_context().expect("context pop should succeed");
}

#[rstest]
fn nested_context_overrides_outer_keys(_isolated_context: ()) {
    push_log_context_map(BTreeMap::from([("user".into(), "outer".into())]))
        .expect("outer context should push");
    push_log_context_map(BTreeMap::from([("user".into(), "inner".into())]))
        .expect("inner context should push");
    let merged = merge_context_values(&BTreeMap::new()).expect("merge should succeed");
    assert_eq!(merged.get("user").map(String::as_str), Some("inner"));
    pop_log_context().expect("inner context should pop");
    pop_log_context().expect("outer context should pop");
}

#[rstest]
fn guards_remove_only_their_own_frames_when_dropped_out_of_order(_isolated_context: ()) {
    let outer = push_log_context([("scope", "outer")]).expect("outer context should push");
    let inner = push_log_context([("scope", "inner")]).expect("inner context should push");

    drop(outer);
    let active = merge_context_values(&BTreeMap::new()).expect("context should merge");
    assert_eq!(active.get("scope").map(String::as_str), Some("inner"));

    drop(inner);
    assert!(
        merge_context_values(&BTreeMap::new())
            .expect("empty context should merge")
            .is_empty()
    );
}

#[rstest]
fn guard_is_not_send_or_sync(_isolated_context: ()) {
    assert_not_impl_any!(LogContextGuard: Send, Sync);
}

#[rstest]
fn foreign_thread_drop_leaves_owner_context_unchanged(_isolated_context: ()) {
    let guard = push_log_context([("request_id", "owner")]).expect("owner context should push");
    let frame_id = guard.frame_id;
    let owner_thread = guard.owner_thread;

    thread::spawn(move || {
        let foreign_guard = LogContextGuard {
            frame_id,
            owner_thread,
            _not_send_or_sync: PhantomData,
        };
        drop(foreign_guard);
    })
    .join()
    .expect("foreign thread should finish");

    let active = merge_context_values(&BTreeMap::new()).expect("context should merge");
    assert_eq!(active.get("request_id").map(String::as_str), Some("owner"));

    drop(guard);
    assert!(
        merge_context_values(&BTreeMap::new())
            .expect("empty context should merge")
            .is_empty()
    );
}

#[rstest]
fn explicit_values_override_context(_isolated_context: ()) {
    push_log_context_map(BTreeMap::from([("request_id".into(), "ctx".into())]))
        .expect("context should push");
    let explicit = BTreeMap::from([("request_id".into(), "inline".into())]);
    let merged = merge_context_values(&explicit).expect("merge should succeed");
    assert_eq!(merged.get("request_id").map(String::as_str), Some("inline"));
    pop_log_context().expect("context should pop");
}

#[rstest]
fn pop_on_empty_stack_errors(_isolated_context: ()) {
    let err = pop_log_context().expect_err("empty pop should fail");
    assert_eq!(err, LogContextError::EmptyContextStack);
}

#[rstest]
fn reject_key_too_long(_isolated_context: ()) {
    let long_key = "k".repeat(MAX_KEY_BYTES + 1);
    let err = push_log_context_map(BTreeMap::from([(long_key.clone(), "v".into())]))
        .expect_err("long key should fail");
    assert_eq!(
        err,
        LogContextError::KeyTooLong {
            key: long_key,
            len: MAX_KEY_BYTES + 1,
            max: MAX_KEY_BYTES,
        }
    );
}

#[rstest]
fn reject_too_many_keys(_isolated_context: ()) {
    let context = (0..=MAX_CONTEXT_KEYS)
        .map(|index| (format!("k{index}"), String::from("v")))
        .collect::<BTreeMap<_, _>>();
    let err = push_log_context_map(context).expect_err("too many keys should fail");
    assert_eq!(
        err,
        LogContextError::TooManyKeys {
            count: MAX_CONTEXT_KEYS + 1,
            max: MAX_CONTEXT_KEYS,
        }
    );
}

#[rstest]
fn reject_value_too_long(_isolated_context: ()) {
    let long_value = "v".repeat(MAX_VALUE_BYTES + 1);
    let err = push_log_context_map(BTreeMap::from([(String::from("ok"), long_value)]))
        .expect_err("long value should fail");
    assert_eq!(
        err,
        LogContextError::ValueTooLong {
            key: String::from("ok"),
            len: MAX_VALUE_BYTES + 1,
            max: MAX_VALUE_BYTES,
        }
    );
}

#[rstest]
fn reject_total_bytes_exceeded(_isolated_context: ()) {
    let value_len = 300usize;
    let mut context = BTreeMap::new();
    for index in 0..60usize {
        context.insert(format!("k{index:02}"), "x".repeat(value_len));
    }
    let err = push_log_context_map(context).expect_err("total bytes limit should fail");
    assert!(matches!(
        err,
        LogContextError::TotalBytesExceeded {
            total,
            max: MAX_TOTAL_BYTES
        } if total > MAX_TOTAL_BYTES
    ));
}
