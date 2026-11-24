//! Compile-pass coverage for the tracing bridge's public const constructor.
//!
//! `layer()` is called from a `const` initializer, so losing its `const`
//! qualifier turns this fixture into a build failure. The crate-private
//! `log_compat` mappers and `FemtoTracingLayer::map_level` cannot be named
//! here, because a UI fixture compiles as an external crate; they carry
//! in-crate `const _` assertions instead.

use _femtologging_rs::tracing_compat::{FemtoTracingLayer, layer};

const LAYER: FemtoTracingLayer = layer();

fn main() {
    // The initializer is the assertion; naming the constant keeps it used.
    let _: FemtoTracingLayer = LAYER;
}