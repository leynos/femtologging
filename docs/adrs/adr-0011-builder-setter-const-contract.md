# Architectural decision record (ADR) 0011: builder setter const and must-use contract

## Status

Accepted — 2026-11-21: keep the handler builders' scalar setters `const` and
`#[must_use]`, pin both qualities with compile-time fixtures, and disclose
rather than suppress the one `missing_const_for_fn` finding that a macro
expansion cannot satisfy.

## Date

2026-11-21

## Context and problem statement

The lint campaign made the handler builders' scalar setters `const fn` and
`#[must_use]`, so a builder chain can be evaluated in a `const` item and a
discarded setter result warns. Neither quality is visible to the run-time test
suite: a `const fn` that quietly loses its qualifier, or a `#[must_use]`
attribute that is removed, still passes every behavioural test.

The Rust setter that `builder_methods!` generates cannot be `const`. Its body
binds `let $self_ident = &mut self;`, and every builder type has a destructor,
so rustc rejects the generated `const fn` with E0493 (rust-lang/rust#133214);
moving `self` into a local before borrowing it is rejected too. The lint fires
once per expansion that could be `const`, and neither `#[expect]` nor
`#[allow]` is acceptable. An `#[expect]` is instantiated once per expansion, so
expansions where the lint does not fire report `unfulfilled_lint_expectations`,
which is fatal under `-D warnings`. An attribute on the `macro_rules!` item
itself does not scope its expansions and is likewise unfulfilled, and
`#[allow]` is barred by the campaign's no-suppression rule and by
`clippy::allow_attributes`.

## Decision drivers

- A compile-time guarantee, not a run-time one, for both qualities.
- No new lint suppressions.
- Fixtures that compile as an external crate, so they fail where the contract
  is broken rather than where the crate happens to be built.
- A recorded disposition for the one finding that cannot be fixed at source.

## Decision

1. The handwritten scalar setters stay `const fn` and `#[must_use]`. The
   `option_setter!` expansion emits both qualifiers. The `builder_methods!`
   Rust setter emits `#[must_use]`; it is not `const`.
2. `rust_extension/tests/ui/pass/builder_const_setters.rs` applies each
   externally nameable `const` chain to a builder passed in as a parameter,
   because every builder constructor allocates and therefore cannot run in a
   `const` item.
3. `rust_extension/tests/ui/fail/unused_socket_http_setters.rs` and the
   pre-existing `unused_overflow_policy.rs` discard a setter result under
   `#![deny(unused_must_use)]`, with committed `.stderr` files.
4. `src/handlers/socket_builder/const_eval_tests.rs` and
   `src/handlers/formatter_id.rs` hold in-crate `const` pins for the accessors
   and setters whose types are not re-exported (`BackoffOverrides` and
   `FormatterId`), which an external fixture cannot name.
5. The `missing_const_for_fn` finding on the generated setter is disclosed in
   the pull request and deferred to the `harden-lint-config` layer, which owns
   the choice between a targeted configuration exception and a macro redesign.
   No `#[allow]` or `#[expect]` is added.

## Consequences

- A setter that loses `const` or `#[must_use]` breaks the build, not a test.
- The trybuild suite gains two fixtures, and their `.stderr` files must be
  refreshed with `TRYBUILD=overwrite` after a rustc or PyO3 change.
- The deferred finding stays visible in the lint tables until
  `harden-lint-config` decides its disposition.

## References

- rust-lang/rust#133214 — E0493 when a generated `const fn` body borrows a
  by-value local.
- [Developers guide](../developers-guide.md), "Handler builder setters:
  `const` and `#[must_use]`".
