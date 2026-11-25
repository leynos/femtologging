//! Compile-pass coverage for the const constructors promoted by the lint pass.
//!
//! `TestConfig::new` is called from a `const` initializer, so losing its
//! `const` qualifier turns this fixture into a build failure. The crate-private
//! helpers the same pass promoted cannot be named here, because a UI fixture
//! compiles as an external crate; they carry in-crate `const _` assertions
//! instead.

use _femtologging_rs::TestConfig;

const DEFAULT_TEST_CONFIG: TestConfig<(), ()> = TestConfig::new((), ());

fn main() {
    assert!(DEFAULT_TEST_CONFIG.capacity > 0);
}