#![cfg(test)]
//! Test helpers for handler builders.
//!
//! This module centralizes repeated assertions used across builder tests.

use super::HandlerBuilderTrait;

/// Assert that building a handler fails.
///
/// Reduces duplication by wrapping the common assertion that
/// `build_inner` returns an error for invalid configurations.
pub fn assert_build_err<B>(builder: &B, msg: &str)
where
    B: HandlerBuilderTrait + ?Sized,
{
    assert!(builder.build_inner().is_err(), "{msg}");
}
