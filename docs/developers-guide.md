# Developers Guide

This guide records project-local development tool choices that must stay
consistent between local `make` targets and CI.

## Ruff version

Ruff is pinned to version `0.16.4` in the Makefile's `RUFF_VERSION`, and
`make lint` invokes it through `uvx ruff==$(RUFF_VERSION)`.
`.github/workflows/ci.yml` repeats the same value as the `RUFF_VERSION` job
environment variable, and `tests/test_lint_version_contract.py` asserts the
Makefile default and the CI environment variable stay identical, without
pinning any specific version itself — a deliberate bump only touches the
Makefile and the workflow file. `ty`, the project's typechecker, is pinned the
same way through `TY_VERSION` (currently `0.0.74`) and checked by the same
contract test.

When updating Ruff, install the intended new version locally, confirm it
resolves through `uvx ruff==<version> --version`, then update `RUFF_VERSION`
in `Makefile` and the matching CI environment variable together. CI does not
install Ruff directly; it picks up the version from the Makefile's
`uvx ruff==$(RUFF_VERSION)` invocation. See [Python
linting](#python-linting) for the complete four-tier lint pipeline.

## Python test toolchain

The Python test toolchain is pinned so local runs and CI use the same pytest
release:

- `pytest` is pinned to `8.4.2` in `pyproject.toml` and CI.
- `pytest-bdd` is pinned to `8.1.0` in `pyproject.toml` and CI.

Keep this policy in sync with `pyproject.toml` and the matching CI install
steps where applicable.

## Typos spelling checker

Markdown spelling is enforced with [`typos`](https://github.com/crate-ci/typos)
so that documentation stays in en-GB-oxendict (Oxford "-ize") spelling.

- `typos` is pinned to `1.48.0`. The pin lives in `TYPOS_VERSION` in the
  `Makefile`, which is the single source of truth; any CI that shells out to the
  `markdownlint` target reuses it, so the Makefile and CI cannot drift apart.
- The `make markdownlint` target runs
  `typos --config typos.toml --force-exclude` across the tracked Markdown files
  after `markdownlint-cli2`.

### Configuration

`typos.toml` is generated, not hand-edited. The generator refreshes the shared
dictionary from `leynos/agent-helper-scripts` into an untracked local cache
only when the authoritative copy is newer. It can reuse a valid cache while
offline, and a clean checkout with an unavailable network retains the reviewed,
tracked `typos.toml` policy.

Put only repository-specific proper nouns, quoted upstream titles, fixtures,
stems or exclusions in `typos.local.toml`, then regenerate the merged config:

```shell
uv run scripts/generate_typos_config.py
```

When updating the pinned version, change `TYPOS_VERSION` in the `Makefile`,
update any matching CI install step, and re-run `make markdownlint` to confirm
the corpus still passes.

## Rust extension build toolchain

The Rust extension build toolchain is pinned so local builds, CI, and the
compatibility tests validate the same maturin and PyO3 releases:

- maturin is pinned to `1.13.3` in the development dependencies and CI build
  steps, with the build-system requirement bounded as `>=1.13.3,<2.0.0`.
- PyO3 is pinned to `0.28.3` in `rust_extension/Cargo.toml`.

When updating either dependency, change the pin in the source manifest, update
the matching CI install step where applicable, and run the maturin/PyO3
compatibility checks through the normal `make test` gate. The synchronization
test in `tests/test_maturin_build.py` verifies that the maturin pins stay
aligned across `pyproject.toml` and `.github/workflows/heavy-tests.yml`.

For `rust_extension/tests/compile_tests.rs`, use
`TRYBUILD=overwrite cargo test --test compile_tests` after rustc or PyO3
changes to refresh `invalid_pymodule_return.stderr` and the other `.stderr`
fixtures.

## Type checking and heavy tests

`make typecheck` depends on `make build`, which creates `./.venv`, installs the
development dependencies, and installs the editable Rust extension there. The
target then invokes:

```shell
ty check --python ./.venv --extra-search-path scripts
```

The `--python` option points `ty` at the project environment containing the
extension and its dependencies. The `scripts` search path is required because
the spelling-policy helpers import one another as top-level modules. The same
paths are recorded in `[tool.ty.environment]` in `pyproject.toml`; the
Makefile passes them explicitly because the pinned local `ty` version does not
reliably apply the equivalent project settings.

The long-running Rust integration suite is the Cargo `heavy` test target,
rooted at `rust_extension/tests/heavy/main.rs`. Its property-based tests are
marked `#[ignore]` because each generated case starts a handler worker. Run
the target explicitly when investigating it:

```shell
cargo test --manifest-path rust_extension/Cargo.toml --no-default-features \
  --test heavy -- --ignored
```

The scheduled `heavy-tests` workflow runs ignored tests across its feature
lanes. Loom model test functions are compiled and registered only when Cargo
is invoked with `--cfg loom`; the ordinary heavy run does not compile or run
them. To select the Loom configuration locally, use:

```shell
RUSTFLAGS="--cfg loom" cargo test --manifest-path rust_extension/Cargo.toml \
  --no-default-features --test heavy
```

The current handlers use `std::thread::spawn`, so executing the Loom models
requires the spawn abstraction described in the heavy-test module
documentation. Until that follow-up is implemented, the Loom configuration
is still compiled to keep the models type-checked.

## Shared Rust test helpers and fixtures

Crate unit-test support is owned by `rust_extension/src/test_utils/` and is
compiled only under `cfg(test)`. Its focused modules keep test arrangement and
assertions reusable without expanding the runtime API:

- `collecting_handler.rs` provides `CollectingHandler`, an in-memory handler
  whose collected records can be inspected by unit tests.
- `frame_test_helpers.rs` provides `StackFrame` factories and assertions for
  frame and payload slices.
- `frame_assertion_helpers.rs` provides pure assertions for required and
  optional frame fields and extracted locals maps. It is available to the
  Python-enabled unit tests.
- `traceback_test_helpers.rs` builds Python-like frame and exception objects,
  arranges extraction cases, and re-exports the frame assertion helpers. It is
  also limited to Python-enabled unit tests.

The reusable integration-test support is owned by
`rust_extension/tests/test_utils/`. It consists of three focused components:

- `handle_expect.rs` defines the `HandleExpect` trait, which turns a handler's
  fallible `handle` call into a descriptive test panic.
- `fixtures.rs` provides `handler_tuple` for a fresh buffer and default
  stream handler, `handler_tuple_custom` for capacity and timeout cases, and
  `stream_handler_for` when several handlers must share one buffer.
- `shared_buffer.rs` provides standard-library and Loom-backed shared buffers;
  use the variant matching the test's execution model.

Each Cargo integration-test root declares only the support modules it needs.
The stream-handler suite includes `test_utils/mod.rs` because it uses all three
components; the file-handler and logger suites include their required files
directly. The `heavy` root includes `shared_buffer.rs` and
`handle_expect.rs` directly, and its Loom modules are themselves gated by
`cfg(loom)`. Prefer these fixtures and the trait over duplicating setup or
`handle(...).expect(...)` calls in individual suites.

File-handler unit tests use `rust_extension/src/handlers/file/test_support.rs`.
The `impl_unsupported_seek!` macro supplies the required `Seek` implementation
for an unseekable test writer and consistently returns
`io::ErrorKind::Unsupported`; the same module also provides the process-wide
test logger and helpers for installing it and taking captured messages.

The macro unit tests in `rust_extension/src/logging_macros.rs` use the
`logger_with_handler` `rstest` fixture. It clears the test logging context,
creates a DEBUG-level `FemtoLogger`, and attaches a `CollectingHandler` so
each macro case can inspect the resulting record.

`rust_extension/src/test_fixtures/explicit_traceback.py` is Python source data,
not a package module. `traceback_capture_tests` embeds it with `include_str!`;
the fixture raises a nested `ValueError`, preserves its explicit traceback,
and clears the exception object's `__traceback__` so tuple-based traceback
capture is exercised.

## Toolchain Boundaries

The root `Makefile` is the source of truth for local and CI tool commands. Keep
new developer tooling behind a Makefile variable or target so CI and local
commands exercise the same path.

Ruff is pinned by `RUFF_VERSION` in the `Makefile`. The `RUFF` variable invokes
`uvx ruff==$(RUFF_VERSION)`, so `make fmt`, `make check-fmt`, and `make lint`
resolve the same formatter and linter version without requiring a global Ruff
install. CI must not add a second hard-coded Ruff installation; update
`RUFF_VERSION` when the project intentionally changes Ruff releases.

The `ty` command no longer needs a separate install: `make typecheck` runs the
pinned release through `uv tool run --from 'ty==$(TY_VERSION)' ty`. CI installs
`uv` and the pinned Makeutil parser, then delegates formatting, linting, type
checking, and tests to Makefile targets.


## Python linting

`femtologging` runs Python linting as four tiers, all reachable through
`make lint` (`lint-python`, followed by `lint-rust`). Each stage must pass
before the next runs. The decision is recorded in
[ADR-005: Four-tier Python lint architecture](adr-005-four-tier-python-lint-architecture.md).

1. **Ruff** — fast, broad rule set in preview mode, targeting `py312`. Pinned
   by `RUFF_VERSION` (`0.16.4`); see [Ruff version](#ruff-version) for the
   pin-sync scheme with CI.
2. **Pylint** (`4.0.7`) — runs through the pinned `leynos/pylint-pypy-shim`
   revision under managed PyPy, isolated from the project virtual
   environment. Configuration lives in `pyproject.toml`'s `[tool.pylint]`
   tables: `py-version = "3.12"`, `max-module-lines = 400`, and a curated
   `enable` list covering logging interpolation, pattern matching, generator
   control flow, environment handling, and subprocess safety.
3. **`df12-python-lints`** and its companion **`ambrleaks`** — run under
   CPython 3.14 so the house-rule parser stays ahead of the project's 3.12
   syntax baseline. `df12-python-lints` is pinned to a specific commit of the
   `v0.3.0` release and enables the message set
   `R9101,C9102,R9103,R9104,C9105,C9106,C9107,R9108,R9109,R9110,R9111,R9112,C9112`.
   `ambrleaks` sweeps Syrupy `.ambr` snapshots under `tests` and
   `femtologging/unittests` for unredacted secrets.
4. **Skylos** (`4.33.2`) — a blocking production dead-code gate, run under
   Python 3.14 so Skylos parses the project's syntax with its own runtime
   `ast` implementation rather than an older one that could produce phantom
   findings. See [Skylos dead-code gate](#skylos-dead-code-gate) below.

Run the full lint gate with:

```shell
make lint
```

To run a single tier locally, invoke the underlying tool directly:

```shell
uvx ruff==0.16.4 check
uv tool run --python pypy --from \
  'git+https://github.com/leynos/pylint-pypy-shim.git@726d09f968b4d729ee4b29c71fc732e744854f3b' \
  --with 'pylint==4.0.7' pylint-pypy femtologging tests scripts
uv tool run --python 3.14 --from 'skylos==4.33.2' skylos \
  --config-file pyproject.toml femtologging \
  --exclude femtologging/unittests --exclude femtologging/_femtologging_rs.pyi \
  --category dead_code --gate --format concise --no-upload --no-provenance \
  --no-grep-verify
```

Prefer running the exact Makefile-derived commands (for example,
`make lint-python`) over hand-copied invocations, since the Makefile is the
single source of truth for tool pins, targets, and flags.


### Skylos dead-code gate

Skylos analyses production code only: `femtologging/unittests` and the native
`femtologging/_femtologging_rs.pyi` stub are excluded, so test-only
references cannot keep a production symbol alive and the stub's inherently
"unused" native parameters never trigger findings. `--no-grep-verify`
prevents a repository-wide text match from masking a genuinely dead
production symbol, and `[tool.skylos.gate] strict = true` in `pyproject.toml`
enforces the strict gate.

Investigate every Skylos finding before suppressing it:

- **Genuine dead code** must be removed.
- A **verified false positive** — an implicit runtime caller such as a
  re-exported native module, a test-util hook, or a protocol-shaped
  parameter — should first be modelled as a typed
  `[[tool.skylos.dead_code.entrypoints]]` rule in `pyproject.toml`, giving the
  fully qualified symbol, its `type` (for example, `"import"`, `"variable"`,
  or `"parameter"`; use `"method"` for methods), and a caller-specific
  reason.
- Only when an entry-point rule cannot describe the boundary should a named
  allow-list exception be recorded:

  ```bash
  make skylos-allow SYMBOL=handler REASON="Loaded by plugin registry"
  ```

  Both `SYMBOL` and `REASON` are required; the target rejects empty or
  whitespace-only values with exit code 2. Use `SYMBOL` rather than `NAME`,
  because Windows Subsystem for Linux (WSL) may inject `NAME` with the host
  name. The target serializes whitelist writes with `flock` against the
  ignored `.skylos-whitelist.lock` file, so concurrent invocations do not
  overwrite one another. Never record a broad or unreasoned exception.

The complete Skylos Makefile contract — the scan command, its exclusions, the
strict gate configuration, the documented-whitelist set, and the entry-point
rule set — is pinned by `tests/test_skylos_lint_contract.py` and
`tests/test_skylos_whitelist_boundary.py`. Both parse the Makefile through
the pinned `makeutil` executable (`makeutil parse Makefile`, emitting JSON)
rather than matching Makefile text, so recording a new exception requires a
conscious update to `tests/test_skylos_lint_contract.py`.


### Makeutil bootstrap

`makeutil` is a prerequisite of `make test` (the `test` target depends on the
`makeutil` target, which only verifies the executable is present) and of
every full-suite CI job — `ci.yml`'s `build-test` job and
`heavy-tests.yml`'s `heavy` job each install their own pinned copy before
running tests. Install the same pinned revision and toolchain locally before
running `make test`:

```bash
rustup toolchain install nightly-2026-05-28 --profile minimal
RUSTFLAGS="-Zpolonius=next" cargo +nightly-2026-05-28 install \
  --git https://github.com/leynos/makeutil \
  --rev 29fc5a1634ffbaa18a773eed9dff1b2838a45d9c \
  --locked --force makeutil
```

## Benchmarking Documentation

Benchmarking work is governed by
[benchmarking-and-optimization-design.md](./benchmarking-and-optimization-design.md)
and tracked in [roadmap.md](./roadmap.md). Keep those links intact when
editing developer workflow notes so contributors can move from toolchain setup
to the benchmarking phase design without losing context.

## Validation

Before committing, run the gates requested by the change. For code changes, the
full local sequence is:

```shell
make check-fmt
make test
make typecheck
make lint
```

For documentation changes, also run:

```shell
make fmt
make markdownlint
make nixie
```

These commands mirror the PR gates described in
[dev-workflow.md](./dev-workflow.md) and keep CI behaviour aligned with local
validation.
