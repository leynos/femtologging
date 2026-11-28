//! Admission classification for the refusal a shard cannot otherwise produce.
//!
//! `Shard::admit` sends only records, so the `UnexpectedJob` arm is defensive:
//! it exists so the match over the job enum stays total. That makes it
//! unreachable through a real send and therefore untestable through the shard.
//! These tests pass the refusal in directly, which is the only way to prove the
//! arm classifies an admission as refused rather than accidentally accepting
//! it, and that it reports rather than failing silently.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::{UNEXPECTED_JOB_DIAGNOSTIC, is_admitted, is_admitted_reporting};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::logger::QueuedRecord;
use crate::logger::dispatcher::AdmissionRefusal;
use crate::logger::dispatcher::Shard;

/// An unexpected job is refused, not accepted.
///
/// This is the whole point of the arm: an admission that comes back carrying
/// something other than a record is a logic error, and the safe direction is to
/// treat the record as dropped. Accepting it would let the caller believe a
/// record was queued when nothing was.
#[test]
fn an_unexpected_job_is_refused() {
    assert!(
        !is_admitted(Err(AdmissionRefusal::UnexpectedJob)),
        "an unexpected job must be classified as a refusal",
    );
}

/// An accepted admission is reported as accepted.
///
/// Asserted beside the refusal so the classifier is pinned in both directions:
/// a function that returned `false` unconditionally would pass the unexpected-job
/// case alone and drop every record in production.
#[test]
fn a_successful_admission_is_accepted() {
    assert!(
        is_admitted(Ok(())),
        "an accepted admission must be classified as accepted",
    );
}

/// The refusal reports through the injected reporter exactly once.
///
/// This is what proves the arm is diagnosable rather than silent. The counter
/// separates "reported" from "not reported at all" and from "reported twice",
/// and the assertion on the return value keeps the report from replacing the
/// refusal classification.
#[test]
fn an_unexpected_job_reports_through_the_reporter_once() {
    let reported = AtomicUsize::new(0);

    let admitted = is_admitted_reporting(Err(AdmissionRefusal::UnexpectedJob), || {
        reported.fetch_add(1, Ordering::SeqCst);
    });

    assert!(!admitted, "an unexpected job must still be refused");
    assert_eq!(
        reported.load(Ordering::SeqCst),
        1,
        "the refusal must report exactly once",
    );
}

/// No other admission outcome reports.
///
/// An accepted admission and a capacity refusal are both ordinary. If either
/// reported, the diagnostic would fire on every busy shard and stop being a
/// signal at all. The capacity refusal is built for real — a shard whose budget
/// admits nothing refuses the very first record — so this exercises the arm a
/// busy process actually takes rather than a synthetic `Err` value.
#[test]
fn ordinary_outcomes_do_not_report() {
    let reported = AtomicUsize::new(0);

    assert!(is_admitted_reporting(Ok(()), || {
        reported.fetch_add(1, Ordering::SeqCst);
    }));
    assert_eq!(
        reported.load(Ordering::SeqCst),
        0,
        "acceptance must not report"
    );

    let shard = Shard::start(0, 8).expect("a test shard must start");
    let budget = Shard::budget_with_limit_for_test(0);
    let refusal = shard
        .admit(queued_record(), &budget)
        .expect_err("a budget of zero must refuse the first record");

    let admitted = is_admitted_reporting(Err(refusal), || {
        reported.fetch_add(1, Ordering::SeqCst);
    });

    assert!(
        !admitted,
        "a capacity refusal must be classified as refused"
    );
    assert_eq!(
        reported.load(Ordering::SeqCst),
        0,
        "a capacity refusal must not use the unexpected-job diagnostic"
    );
}

/// Build one queued record for a shard that will never dispatch it.
fn queued_record() -> Box<QueuedRecord> {
    Box::new(QueuedRecord {
        record: FemtoLogRecord::new("admission.tests", FemtoLevel::Info, "refused"),
        handlers: Vec::new(),
        #[cfg(feature = "python")]
        context: None,
    })
}

/// The diagnostic names the operation and the refusal category.
///
/// The wording is the contract an operator alerts on, so it is pinned rather
/// than left to drift with the surrounding prose. Both fields are asserted
/// because a message carrying only one of them cannot be filtered.
#[test]
fn the_diagnostic_names_the_operation_and_the_category() {
    assert!(
        UNEXPECTED_JOB_DIAGNOSTIC.contains("operation=producer_queue_dispatch"),
        "the diagnostic must name the operation: {UNEXPECTED_JOB_DIAGNOSTIC}",
    );
    assert!(
        UNEXPECTED_JOB_DIAGNOSTIC.contains("refusal=unexpected_job"),
        "the diagnostic must name the refusal category: {UNEXPECTED_JOB_DIAGNOSTIC}",
    );
    assert!(
        !UNEXPECTED_JOB_DIAGNOSTIC.contains("capacity"),
        "the diagnostic must not reuse the capacity wording: {UNEXPECTED_JOB_DIAGNOSTIC}",
    );
    assert!(
        !UNEXPECTED_JOB_DIAGNOSTIC.contains('\n'),
        "the diagnostic must be one line: {UNEXPECTED_JOB_DIAGNOSTIC}",
    );
}
