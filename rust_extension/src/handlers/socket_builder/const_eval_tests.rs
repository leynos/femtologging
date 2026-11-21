//! Compile-time checks for the backoff overrides' `const` surface.
//!
//! `BackoffOverrides` is `pub` inside the crate but is not re-exported, so an
//! external `trybuild` fixture cannot name it. This module is therefore the
//! enforcing test for its `const` accessors and setters, and for both builders'
//! `with_backoff` setters, which take one. A run-time test cannot detect a lost
//! `const` qualifier. The externally nameable builder setters are pinned by
//! `tests/ui/pass/builder_const_setters.rs`.

use crate::handlers::http_builder::HTTPHandlerBuilder;

use super::{BackoffOverrides, SocketHandlerBuilder};

/// An override set built entirely in const context.
///
/// The struct literal replaces `new()`, which delegates to the derived
/// `Default` and therefore cannot run in a const item.
const OVERRIDES: BackoffOverrides = BackoffOverrides {
    base_delay: Some(100),
    max_delay: Some(200),
    reset_after: Some(300),
    deadline: Some(400),
};

// The four getters must stay callable in const context.
const _: [Option<u64>; 4] = [
    OVERRIDES.base_ms(),
    OVERRIDES.cap_ms(),
    OVERRIDES.reset_after_ms(),
    OVERRIDES.deadline_ms(),
];

// The four fluent setters must stay callable in const context.
const OVERRIDDEN: BackoffOverrides = OVERRIDES
    .with_base_ms(1)
    .with_cap_ms(2)
    .with_reset_after_ms(3)
    .with_deadline_ms(4);

const fn socket_backoff_chain(builder: SocketHandlerBuilder) -> SocketHandlerBuilder {
    builder.with_backoff(OVERRIDES)
}

const fn http_backoff_chain(builder: HTTPHandlerBuilder) -> HTTPHandlerBuilder {
    builder.with_backoff(OVERRIDDEN)
}

// Naming each `const fn` keeps it from being reported as dead code; the
// `const` qualifier on its definition is the assertion.
const _: fn(SocketHandlerBuilder) -> SocketHandlerBuilder = socket_backoff_chain;
const _: fn(HTTPHandlerBuilder) -> HTTPHandlerBuilder = http_backoff_chain;
