//! Admission classification for the refusal a shard cannot otherwise produce.
//!
//! `Shard::admit` sends only records, so the `UnexpectedJob` arm is defensive:
//! it exists so the match over the job enum stays total. That makes it
//! unreachable through a real send and therefore untestable through the shard.
//! These tests pass the refusal in directly, which is the only way to prove the
//! arm classifies an admission as refused rather than accidentally accepting
//! it.

use super::is_admitted;
use crate::logger::dispatcher::AdmissionRefusal;

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
