//! Compile-time pins for the producer's `const fn` surface.
//!
//! `level_from_raw` decodes the logger's atomic level representation, and it is
//! `const` so a caller can fold the decode. It is crate-private, so an external
//! `trybuild` fixture cannot name it — a UI fixture compiles as its own crate.
//! A compile-time binding here is therefore the enforcing test: a `const fn`
//! that quietly loses its qualifier still passes every behavioural test.
//!
//! The pattern follows `crate::log_compat::const_eval_tests`.

use crate::level::FemtoLevel;

use super::FemtoLogger;

/// Every raw value maps to the level it names, and an out-of-range value is
/// treated as the most severe rather than panicking.
///
/// The mapping is asserted here, beside the pin, so the two guarantees are read
/// together: the function must stay const-evaluable *and* keep decoding the
/// stored representation the same way.
#[test]
fn the_raw_level_decode_is_total_and_ordered() {
    const TRACE: FemtoLevel = FemtoLogger::level_from_raw(0);
    const DEBUG: FemtoLevel = FemtoLogger::level_from_raw(1);
    const INFO: FemtoLevel = FemtoLogger::level_from_raw(2);
    const WARN: FemtoLevel = FemtoLogger::level_from_raw(3);
    const ERROR: FemtoLevel = FemtoLogger::level_from_raw(4);
    const CRITICAL: FemtoLevel = FemtoLogger::level_from_raw(5);
    const OUT_OF_RANGE: FemtoLevel = FemtoLogger::level_from_raw(u8::MAX);

    assert_eq!(
        [TRACE, DEBUG, INFO, WARN, ERROR, CRITICAL],
        [
            FemtoLevel::Trace,
            FemtoLevel::Debug,
            FemtoLevel::Info,
            FemtoLevel::Warn,
            FemtoLevel::Error,
            FemtoLevel::Critical,
        ],
        "each raw value must decode to the level it names",
    );
    assert_eq!(
        OUT_OF_RANGE,
        FemtoLevel::Critical,
        "an out-of-range raw value must decode to the most severe level",
    );
}
