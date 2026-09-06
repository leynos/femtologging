# Architectural decision record (ADR) 005: environment seam taxonomy

## Status

Accepted.

## Date

2026-09-06

## Context and problem statement

The Rust extension reads no process environment variable today. Nothing in
`rust_extension/src`, `rust_extension/tests`, or `rust_extension/benches` calls
`std::env::var`, `var_os`, `vars`, `vars_os`, `set_var`, or `remove_var`. That
is a property worth keeping rather than an accident worth leaving unguarded.

The property matters most for the test suite. A test that mutates the parent
process environment mutates it for every other test in the same process, so a
suite containing one such test has to serialize around it. `make test` already
runs Cargo with `--test-threads=$(TEST_THREADS)`, defaulting to one thread, and
the crate carries `serial_test` for the tests that coordinate the global logging
manager and the Python interpreter. Adding environment mutation to that list
would add a second, unrelated reason to serialize and would make restoring
parallel execution harder later. Ambient reads have the mirror-image problem:
a value the caller cannot supply is a value the test cannot vary.

`leynos/netsuke` solved this with a Clippy `disallowed-methods` policy plus a
stated taxonomy of injection shapes, recorded in its ADR 008. Adopting the same
policy here keeps the two repositories reviewable against one yardstick.

Two related pieces of lint work are still open: issue #420 aligns the crate's
whole lint baseline with Netsuke's, and issue #421 centralizes the feature-lane
matrix and adds rustdoc. This ADR depends on the narrow parts of both — a
`deny` severity and a lint that reaches test code — and this decision installs
those parts directly rather than waiting.

## Decision

Ban ambient process-environment access in the Rust extension and choose an
injection shape by the size of the boundary.

### Enforcement

`rust_extension/clippy.toml` disallows all six methods. Each entry carries the
remedy, because Clippy prints the reason in the diagnostic:

| Method | Reason printed |
| --- | --- |
| `std::env::var` | inject an environment reader |
| `std::env::var_os` | inject an environment reader |
| `std::env::vars` | inject an environment reader |
| `std::env::vars_os` | inject an environment reader |
| `std::env::set_var` | use a stub environment in tests |
| `std::env::remove_var` | use a stub environment in tests |

`rust_extension/Cargo.toml` denies `clippy::disallowed_methods` in its
`[lints.clippy]` table, so the severity travels with the crate rather than
living in one Clippy invocation.

The `lint-env-policy` Make target then runs that lint over the whole crate:

```make
ENV_POLICY_CLIPPY_FLAGS ?= --all-targets --all-features -- -A clippy::all -D clippy::disallowed_methods
```

`--all-targets --all-features` reaches integration tests, benches, and every
feature combination in one pass. `-A clippy::all` is deliberate and temporary:
the existing feature lanes in `lint-rust` omit `--all-targets` because the test
tree carries a backlog of unrelated Clippy findings, and clearing that backlog
is issue #421's job. Silencing the rest of Clippy in this one lane lets the
environment policy govern test code today without absorbing that work. Once
issue #421 lands, these flags fold into its lane list and this lane retires.

### Seam selection

Choose the lightest shape the boundary justifies:

- **An explicit value**, for one-off configuration. The caller passes the
  resolved value as an argument. Nothing behind the boundary knows the value
  came from an environment variable.
- **A narrow reader closure**, for a small reusable boundary. The module owns a
  private function taking an `FnOnce(&str) -> Result<String, env::VarError>`
  (or the `OsString`-typed equivalent) instead of reading the process itself.
- **A shared environment trait**, only when several variables and several tests
  justify one. A trait for a single-variable, single-caller site recreates the
  ambient coupling the seam exists to remove, one layer down.

A direct read is permitted only at a genuine executable composition root, and
only under an item-scoped attribute:

```rust
#[expect(clippy::disallowed_methods, reason = "composition root: <what and why>")]
```

`expect` rather than `allow` is deliberate. The expectation goes unfulfilled,
and therefore warns, once the site is migrated, so the backlog removes itself
instead of rotting. The Rust extension is loaded as a Python extension module
and has no `main`, so it has no composition root of its own today; the attribute
exists for a future binary target.

### Tests

A test never mutates the parent process environment. Where a test needs a child
process to see a variable, it builds the child's environment explicitly with
`Command::env` (and `Command::env_clear` where isolation matters) rather than
setting the variable in the harness and letting the child inherit it. Where a
test needs the code under test to observe a value, it supplies that value
through the seam.

Serialization is not an accepted substitute. A `#[serial]` attribute or a
reduced `TEST_THREADS` value stays only where a genuine structural reason
demands it, such as the process-global logging manager or the Python
interpreter state, and never as compensation for environment mutation.

### Scope

This ADR governs `rust_extension` only. The Python package's configuration
access is governed by the Python architecture and its own lint stack.

## Consequences

- A contributor who reaches for `std::env` gets a build failure naming the
  remedy, in every feature lane and every target kind.
- `rust_extension/tests/env_access_policy.rs` fails if any of the six entries
  leaves `clippy.toml`, if the manifest stops denying the lint, or if the
  policy lane stops covering every target and feature or drops out of
  `make lint`. Each of its assertions records the mutation that proved it.
- Adding a new environment-dependent boundary means choosing among three named
  shapes, so a reviewer can reject one that is heavier or lighter than the
  call-site count warrants.
- The suite keeps only the serialization it structurally needs, leaving the
  route to parallel test execution open.
- The contract test reads three checked-in configuration files, one of which
  sits above the crate directory, so its crate joins the
  `no_std_fs_operations` exclusion list in `rust_extension/dylint.toml` with a
  rationale. Whitaker ignores in-source suppression for that lint.

## Alternatives considered

- **Documenting the rule without enforcing it.** Rejected: the property being
  preserved is the absence of something, and prose cannot detect the first
  reintroduction. The audit that found the zero-state would have to be repeated
  by hand on every change.
- **A single shared environment trait for every boundary.** Rejected for the
  same reason Netsuke rejected it: indirection with no matching test-surface
  benefit at single-variable sites.
- **Allowing environment mutation in tests behind `#[serial]`.** Rejected: it
  trades a compile-time guarantee for a convention that only holds while every
  future author remembers it, and it entrenches serialization the suite is
  trying to shed.
- **Waiting for issues #420 and #421.** Rejected: the zero-state is guarded
  cheaply now, and both issues subsume this configuration rather than
  conflicting with it.

## References

- Netsuke seam taxonomy:
  <https://github.com/leynos/netsuke/blob/main/docs/adr-008-environment-seam-taxonomy.md>
- Netsuke Clippy configuration:
  <https://github.com/leynos/netsuke/blob/main/clippy.toml>
- Contract test: `rust_extension/tests/env_access_policy.rs`
