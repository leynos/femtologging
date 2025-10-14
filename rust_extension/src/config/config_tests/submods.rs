//! Grouped configuration tests split by domain concerns.
#![cfg(all(test, feature = "python"))]

mod validation_tests;
mod propagation_tests;
mod disable_existing_tests;
