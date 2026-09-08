# Architectural decision record (ADR) 006: environment seam taxonomy

## Status

Accepted — 2026-09-06: ban ambient process-environment access in the Rust
extension, enforce it with Clippy, and require an injected seam in its place.

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

The `lint-env-policy` Make target then runs that lint over the whole crate,
one Cargo invocation per feature selection:

```make
ENV_POLICY_FEATURE_LANES ?= none extension-module python test-util log-compat tracing-compat all
ENV_POLICY_CARGO_ARGS ?= --all-targets
ENV_POLICY_LINT_ARGS ?= -A clippy::all -D clippy::disallowed_methods
```

`--all-targets` reaches integration tests and benches. The lane list is what
reaches every feature arm. `none` maps to `--no-default-features`, `all` maps
to `--all-features`, and every other entry maps to `--no-default-features
--features <name>`, so a named lane enables exactly one feature and nothing
else. `--all-features` alone would never compile a `#[cfg(not(feature = ...))]`
block, and the crate has such blocks; `none` and `all` between them compile
both arms of every single-feature gate, and each named lane compiles that
feature's code with the others absent. Intermediate combinations are not
linted, which is a deliberate limit: the policy bans a call outright rather
than under a condition, so a violation cannot be reachable only under a
combination and not under a lane that enables the feature it sits behind.

The lanes are walked by `scripts/lint_rust_lanes.py`, not by a shell loop in
the Makefile. This is the second attempt at that walk and the reason for the
change is worth recording. The first attempt was a `for` loop in the recipe,
and a shell `for` loop reports the status of its last command, so a rejection
in any earlier lane was discarded and the target exited 0. Only the `all` lane
could fail it, which is the wrong one: `none` is the only lane that compiles a
`#[cfg(not(feature = ...))]` block and it runs first. The gate did not gate.
A `|| exit 1` guard on each call fixes it, but that guard is one character
short of absent, invisible in review, and its loss looks exactly like success.

Moving the walk into a script per the estate's scripting standards makes the
failure path ordinary code with ordinary tests:
`scripts/tests/test_lint_rust_lanes.py` covers a failing first lane, a failing
last lane, and the all-pass case with `cmd-mox` shimming `cargo`, so no Rust is
compiled to exercise them. The script stops at the first failing lane and names
it, so the log ends at the lane a contributor has to reproduce. `make lint`
runs those tests before it trusts the driver.

`-A clippy::all` is deliberate and temporary: the lanes in `lint-rust` omit
`--all-targets` because the test tree carries a backlog of unrelated Clippy
findings, and clearing that backlog is issue #421's job. Silencing the rest of
Clippy in this one target lets the environment policy govern test code today
without absorbing that work. Once issue #421 lands, these settings fold into
its lane list.

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

`allow` is not an alternative anywhere, and a source scan enforces that. Every
Rust source is parsed and every attribute walked, following `cfg_attr`, and no
source may allow `clippy::disallowed_methods`, the `clippy::style` group that
contains it, the wider `clippy::all`, or `warnings`, inner or outer.

A crate-level `#![allow(...)]` is the dangerous shape, because it disables the
policy for a whole crate while the Clippy configuration, the manifest severity,
the lane list and the compiled fixture all stay exactly as they are, and
`clippy::allow_attributes` does not fire on inner attributes. Measured: with
that one line added and a real `std::env::var` call beneath it, every other
contract stayed green and the policy lane exited 0.

Nor is writing the attribute where a reader can see it. An `allow` emitted
from a `macro_rules!` arm is expanded and honoured by Clippy while `syn` keeps
the arm's body an opaque token stream, so the scan walks macro token streams as
well as parsed attributes. Measured: a macro arm emitting
`#[allow(clippy::disallowed_methods)]` around a `std::env::var` call reports
zero diagnostics where the same file without it reports one.

Naming the lint is not required to silence it either, which is why the scan
parses rather than searches. Measured against Clippy 0.1.98, each on a probe reporting
one diagnostic without an attribute: `#![allow(clippy::style)]` and
`#![allow(clippy::all)]` each reduce it to none, as does
`#![cfg_attr(all(), allow(clippy::disallowed_methods))]`, which is honoured by
Clippy, unreported by `clippy::allow_attributes`, and invisible to a scan
looking for a line that begins with an attribute. `warnings` silences the lint
only where it sits at its configured level; the policy lane denies it on the
command line, so `warnings` does not evade that lane. It is guarded anyway,
against the manifest severity ever softening.

`expect` rather than `allow` is deliberate. The expectation goes unfulfilled,
and therefore warns, once the site is migrated, so the backlog removes itself
instead of rotting.

The Rust extension is loaded as a Python extension module and has no `main`,
so it has no composition root of its own. It has one all the same, in the
tests: the point where a child process's environment is built. Clearing that
environment is what makes a subprocess test hermetic, and a cleared
environment has no `PATH`, so the child could not be found at all. That single
read of the parent's `PATH`, at the site where the child's environment is
composed, is the one sanctioned exception, and it carries the item-scoped
attribute like any other. `option_env!` is not a way around it: it resolves at
compile time and would bake in the building machine's `PATH`.

The exception is narrow on purpose. It covers the value a cleared environment
cannot supply and nothing else; every other variable the child needs is set
explicitly, and a second read for a different variable would be a new decision
rather than an extension of this one.

### Tests

A test never mutates the parent process environment, and never reads it
ambiently either. Where a test needs a child process to see a variable, it
builds the child's environment explicitly rather than setting the variable in
the harness and letting the child inherit it. `tests/env_access_policy.rs`
holds itself to this: its `clippy-driver` probe clears the environment and
adds back only `CLIPPY_CONF_DIR` and `PATH`, so the diagnostics it counts
cannot be perturbed by an inherited `RUSTFLAGS`, `CLIPPY_ARGS`, or a
`CLIPPY_CONF_DIR` pointing at another configuration.
`Command::env` alone leaves the parent's environment in place, so a test whose
outcome could depend on an inherited variable calls `Command::env_clear` first
and then adds back every variable the child legitimately needs, `PATH`
included. Where a
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
  policy target stops covering every target and feature, stops failing when a
  lane fails, or drops out of `make lint`. The failure path is held at two
  levels: the driver's own unit tests, and a test that runs the real
  `make lint-env-policy` with a failing lane ahead of a passing one. It also compiles
  `tests/fixtures/env_policy_probe.rs` through `clippy-driver` under this
  crate's `clippy.toml` and checks that all six methods are rejected, that each
  diagnostic carries the remedy this ADR promises, and that the
  composition-root `expect` suppresses exactly one call. Each of its assertions
  records the mutation that proved it.
- Adding a feature to the crate manifest without adding a lane fails that test,
  so the lane list cannot drift behind the feature list.
- Adding a new environment-dependent boundary means choosing among three named
  shapes, so a reviewer can reject one that is heavier or lighter than the
  call-site count warrants.
- The suite keeps only the serialization it structurally needs, leaving the
  route to parallel test execution open.
- The contract test embeds the three checked-in configuration files with
  `include_str!`, which resolves relative to the test source at compile time
  and so reaches the `Makefile` above `CARGO_MANIFEST_DIR` without any runtime
  filesystem access. Whitaker's `no_std_fs_operations` therefore never fires
  and the crate needs no `dylint.toml` exclusion, while moving or deleting one
  of the three files becomes a compile error rather than a runtime failure.

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
