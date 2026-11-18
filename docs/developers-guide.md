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
resolves through `uvx ruff==<version> --version`, then update `RUFF_VERSION` in
`Makefile` and the matching CI environment variable together. CI does not
install Ruff directly; it picks up the version from the Makefile's
`uvx ruff==$(RUFF_VERSION)` invocation. See [Python linting](#python-linting)
for the complete five-tier lint pipeline.

## Python test toolchain

The local development group in `pyproject.toml` is the single source of the
test toolchain, and the scheduled `.github/workflows/heavy-tests.yml` job
installs that group from `uv.lock` with `uv sync --frozen --group dev` rather
than repeating the pins. The sync reads `.github/uv-test-util.toml` through
`UV_CONFIG_FILE`, so the project build receives the `test-util` hooks, and the
job then runs `maturin develop` against the synced environment. A dependency or
version-policy change therefore belongs in the development group only:

- `maturin[patchelf]` is pinned to `1.13.3` for the editable Rust extension
  build, matching the build-system lower bound.
- `pytest` is pinned to `9.1.1`.
- `pytest-bdd` is pinned to `9.0.0`.
- `pytest-timeout`, `pytest-xdist`, and `syrupy` are currently unpinned; they
  follow the resolver's latest compatible releases, which the lock records.
- `hypothesis` is bounded to `>=6.135,<7`, so property-based and
  generated-input coverage only shifts with a deliberate compatible-version
  change.
- `pyyaml` is bounded to `>=6.0.3,<7` because the lint contract tests parse
  the CI workflow files.

### Stack-info snapshot normalization

`stack_info=True` records capture the whole Python call stack, so a snapshot
would otherwise record pytest, pluggy and interpreter frames alongside this
project's. `normalize_traceback_output()` alone cannot make that stable: it
masks paths and line numbers but keeps each frame's source text, and an
upstream release may wrap a call across lines. The snapshot step therefore calls
`strip_external_stack_frames()` in `tests/steps/stack_frames.py` first.

Stripping has a deliberately narrow scope. It acts only inside a
`Stack (most recent call last):` section, up to the next
`Traceback (most recent call last):` heading or the end of the output, so an
exception traceback is never edited. A frame is external when its filename is a
`<frozen …>` pseudo-file or resolves beneath one of the `stdlib`, `platstdlib`,
`purelib`, `platlib` or `scripts` entries of `sysconfig.get_paths()`. A removed
frame takes its whole block with it: every source line of a wrapped statement
and every caret line. Relative and unrecognized filenames are kept, because
they cannot be attributed to installed code reliably.

The function is pure and idempotent, and it deletes whole lines only: each
surviving line keeps the terminator it already had. An output that uses `\r\n`
is returned with `\r\n`, so a traceback stays byte-identical. The
launcher-frame rule in `tests/steps/traceback_normalization.py` is separate and
runs after stripping; a `runpy.py` counts as launcher code only when its own
directory is an interpreter directory inside `lib` or `lib64`, or a `Lib`
directory one level above one, so a project's own `runpy.py` keeps its frame.

`tests/steps/test_external_stack_frames.py` pins the named cases, including
CRLF preservation, and `tests/steps/test_stack_frames_properties.py` states the
invariants as Hypothesis properties: stripping removes only whole lines, is
idempotent, preserves an exception tail byte for byte, removes a frame beneath
any supplied root and keeps a relative frame. The stack-info feature scenario
also asserts that the raw output contains the `log_with_stack_info` frame, so a
stripping rule that emptied the snapshot could not pass silently.

### Docstring examples

`make test` runs pytest with `--doctest-modules`, so every `>>>` example in a
module pytest walks is executed as part of the suite. Before it was wired the
suite collected 621 items and no examples; with it, 655. The 34 new items are
the examples, and five of them were failing: `poll_file_for_text`,
`capture_records`, `latest_key_values`, `AdapterProbe.capture` and
`_sole_emitted_record` each opened with a name nothing bound, so each raised on
its first execution. An example nobody executes is a comment that looks like
evidence.

Two rules follow from that.

Write an example so it runs on its own. Bind every name it uses, and show a
value rather than an assertion, so a wrong answer is a failure and not a silent
pass.

Skip an example only with a written reason beside it. `capture_records` used to
be skipped because its process-wide macro emission needed a bounded wait for a
Python handler, and `flush_handlers()` held the GIL during that wait
([issue 451](https://github.com/leynos/femtologging/issues/451)). The flush
barrier now releases the GIL, so the example exercises enqueue, flush, and
inspection directly without waiting on a delivery race. An example that waits
on a race is the shape
[issue 476](https://github.com/leynos/femtologging/issues/476) is about. A
skipped example is inert text, never compiled, so it cannot be trusted the way
an executed one can.

`--doctest-modules` imports every module it walks, not only `test_*.py`.
`scripts/conftest.py` excludes `scripts/lint_rust_lanes.py`, whose dependencies
live in its own `uv` script block and cannot be resolved from the project
environment; `make lint-lanes-test` runs that script, and its examples, in the
environment that can.

### Documented examples

User-facing documents mark each tested fence with
`<!-- tested-example: <identifier> -->` on the line directly above it. Python
tests load examples through one shared loader,
`tests/documentation_examples.py`, so a test runs the shipped text rather than
a copy of it. Rust examples in the users' guide take a separate route, because
they must become Rust source: `rust_extension/build.rs` writes each marked Rust
fence to `rust_extension/tests/generated/`, where an integration test
`include!`s it.

The Python loader reads `README.md`, `docs/users-guide.md`, and
`docs/v0-2-0-migration-guide.md`:

- `documented_example(identifier)` returns one example, with its language,
  exact body, document, and line. `load_documented_examples()` returns all of
  them, and `run_example(example, cwd)` runs a Python example in a fresh
  interpreter.
- `scan_document(source, text)` enforces the marker contract. It raises
  `DocumentationExampleError` for a malformed marker or identifier, a marker
  with anything but a fence on the next line (a blank line included), a marked
  fence without a language, an unterminated fence, or a repeated identifier.
- Only a bare run of the fence character, indented by at most three spaces,
  closes a fence. A fence line that carries an info string, such as a `python`
  tag, stays in the body, as CommonMark specifies.
- A fence without a marker is not an error to the loader. `unmarked_fences()`
  reports each one with its location, so a test can require every fence in a
  document to be marked once that document has been brought up to the
  [documentation style guide](documentation-style-guide.md).

`tests/test_documentation_examples_loader.py` pins these rules with named and
parameterized cases, and `tests/test_documentation_examples_loader_property.py`
states the closing-fence length and indentation rules and body preservation as
Hypothesis properties, so the rules hold across fence lengths and body text
rather than only at the cases written down. `tests/test_documented_examples.py`
runs the users' guide's `flush-handlers-usage` example and checks that the
record it logs reached the file it flushes.

## Typos spelling checker

Markdown spelling is enforced with [`typos`](https://github.com/crate-ci/typos)
so that documentation stays in en-GB-oxendict (Oxford "-ize") spelling. The
configuration is built by
[`typos-config-builder`](https://github.com/leynos/typos-config-builder), which
`TYPOS_CONFIG_BUILDER` in the `Makefile` pins to release `v0.1.3`.

The builder requires Python 3.14 or newer, so `TYPOS_CONFIG_BUILDER` pins
`--python 3.14` as well. `uvx` otherwise honours `UV_PYTHON`, which the hosted
matrix exports for each lane it tests, and the 3.12 and 3.13 lanes would hand
the builder an interpreter it refuses; an explicit `--python` outranks the
variable. Keep the pin when the matrix changes, and keep it in the standalone
command below so the two invocations stay identical. The decision, and the
alternatives that were rejected, are recorded in
[ADR 0010](adrs/adr-0010-spelling-config-builder.md).

- `make spelling` runs the pinned `typos-config-builder gate`. The gate
  refreshes the shared en-GB-oxendict dictionary from
  `leynos/agent-helper-scripts` on its `main` branch, merges the repository
  overlay, writes `typos.toml`, then checks Markdown spelling and the shared
  phrase corrections. Both stages always run, so a phrase finding cannot hide a
  spelling finding.
- The builder is release-pinned while the dictionary it fetches stays live:
  either half can move without repinning the other. The gate caches the
  dictionary revision in the untracked `.typos-oxendict-base.toml` and
  `.typos-oxendict-base.json` files, and a clean checkout fetches it again.
- The gate runs in write mode, so a live dictionary edit is picked up rather
  than reported as drift. When `make spelling` changes `typos.toml`, commit the
  regenerated file with the change that caused it.
- `make markdownlint` depends on `make spelling`, so Markdown structure and
  spelling are checked in one pass.

CI runs the same `make spelling` target, so a local pass and a hosted pass use
identical inputs. The gate is deliberately not invoked with `--check` there:
that option only reports drift, and the supported CI behaviour is to regenerate
and then check.

### Configuration

`typos.toml` is generated: never edit it by hand. Put narrow
repository-specific exceptions in `typos.local.toml`, scoped by pattern rather
than by accepted word:

- `[patterns] ignore` entries match exact identifiers, quotations or fixture
  lines;
- `[oxford] stems` accept the inflections of a proper noun that the shared
  dictionary spells differently;
- `[files] exclude` removes generated trees from the scan.

The overlay also supports `[patterns] remove` and `[patterns] markdown_only`
when a shared pattern is too broad for this repository. Run the gate directly
to regenerate the file after changing the overlay:

```shell
uvx --python 3.14 \
  --from "git+https://github.com/leynos/typos-config-builder.git@v0.1.3" \
  typos-config-builder gate
```

## Rust extension build toolchain

The Rust extension build toolchain is pinned so local builds, CI, and the
compatibility tests validate the same maturin and PyO3 releases:

- `rust-toolchain.toml` selects Rust `1.99.1` exactly and requires `rustfmt`,
  `clippy`, and `rust-analyzer`. A floating `stable` channel is not used,
  because rustup would resolve it at install time and the toolchain would
  change without a commit. Dependabot's `rust-toolchain` update entry in
  `.github/dependabot.yml` proposes each new release as a pull request, which
  passes the usual gates before the pin moves. Install the toolchain with:

  ```shell
  rustup toolchain install 1.99.1 --component rustfmt --component clippy \
    --component rust-analyzer
  ```

  Cargo commands run from the repository root select that toolchain
  automatically. `make check-fmt` and `make lint` use its formatter and linter;
  editor integrations use the installed language server.
- maturin is pinned to `1.13.3` in the development dependencies and CI build
  workflows, with the build-system requirement bounded as `>=1.13.3,<2.0.0`.
- PyO3 is pinned to `0.30.1` in `rust_extension/Cargo.toml`.
- The `rust_extension` crate sets `publish = false`: it ships as a wheel built
  by maturin, never through `cargo publish`, so `cargo publish` refuses it and
  the crate declares no minimum supported Rust version.

The Rust extension also sets `missing_docs = "deny"` in
`rust_extension/Cargo.toml`, so every public item needs a doc comment in every
build, not just under Clippy; for PyO3 classes and methods that comment is the
Python docstring. It denies `clippy::missing_docs_in_private_items`, so private
items and Rust test support must have meaningful documentation alongside the
public API. This includes integration tests, benchmarks, unit-test modules, and
feature-gated compatibility code. `make lint` runs Clippy with `-D warnings` and
`--all-targets` across every supported Rust feature lane, so each lane applies
the same documentation and warning policy. Because Clippy skips private-item
documentation checks for code compiled in test mode, `lint-rust-test-docs` also
parses test-support source and requires Rustdoc on helper types and their named
fields. It is part of `lint-rust`; compiler UI fixtures remain intentionally
minimal and are excluded from that source-level scan.

When updating either dependency, change the pin in the source manifest, update
the matching CI install step where applicable, and run the maturin/PyO3
compatibility checks through the normal `make test` gate. The synchronization
test in `tests/test_maturin_build.py` verifies that the maturin pins stay
aligned across `pyproject.toml` and `.github/workflows/build-wheels.yml`. The
isolated sdist builders read the same pin from
`.github/maturin-build-constraints.txt` through `UV_BUILD_CONSTRAINT`; update
that file alongside the other pins.
`tests/test_ci_build_wheels_sdist_cardinality.py::test_sdist_builds_share_the_pinned_maturin_constraint`
verifies the sdist constraint.

For `rust_extension/tests/compile_tests.rs`, use
`TRYBUILD=overwrite cargo test --test compile_tests` after rustc or PyO3
changes to refresh `invalid_pymodule_return.stderr` and the other `.stderr`
fixtures.

### Maturin build features

The persistent `features = ["python"]` setting in the `[tool.maturin]` table is
the production build configuration. It keeps the test-only `test-util` feature
and timed-rotation hooks out of ordinary wheel builds.

Tests that exercise those hooks opt in explicitly. `make build` and the heavy
test workflow run `maturin develop --features python,test-util`. The coverage
action uses `.github/uv-test-util.toml` through `UV_CONFIG_FILE` so its source
install receives the same features without changing `pyproject.toml`. Coverage
and heavy-test runs set `FEMTOLOGGING_EXPECT_TEST_UTIL=1`; `tests/conftest.py`
then fails before tests start if the extension omitted the hooks.

### Documentation example loader

`docs/documentation-style-guide.md` requires user-facing examples to be
compiled as published, not as a copy kept beside them. `build.rs` therefore
reads `docs/users-guide.md` and extracts every fenced block introduced by a
`<!-- tested-example: <identifier> -->` marker into
`rust_extension/tests/generated/`. The identifiers use lowercase letters,
digits, and dashes; each generates one module named after the identifier with
dashes replaced by underscores. `tests/process_payload_docs.rs` `include!`s the
generated module, so `make test` compiles exactly the guide text.

The loader is strict. An unmarked Rust fence, an unterminated fence, and a
marker that is malformed, unnamed, repeated, or not followed by a fence all
fail the build; a warning would leave a guide example untested. Fences in other
languages are skipped whole, so a Rust fence quoted inside one is content
rather than a second example.

The Markdown rules live in `rust_extension/tests/test_utils/doc_examples.rs`
and the publication and pruning of generated modules in
`rust_extension/tests/test_utils/generated_files.rs`. Both modules are
`#[path]`-included by `build.rs` and by the integration tests, so the code that
runs during a build is the code the tests exercise:
`tests/doc_example_loader.rs` covers the fence rules, including property tests,
and `tests/generated_files.rs` covers writing, replacing, and pruning modules
through a capability handle.

The generated files are written inside `tests` rather than to `OUT_DIR` because
the environment-policy source scan refuses any `include!` target the scan never
walks; build output could carry a lint suppression the scan would never see,
and that refusal is deliberately not weakened for documentation. The directory
is ignored by Git, so regenerate it by rebuilding. The build script needs
`camino` and `cap-std`, which are build-time dependencies only. A source
distribution must carry `docs/users-guide.md`, which is why `pyproject.toml`
adds it to the sdist include list.

### Rust log bridge and handler waits

With `log-compat`, each logger shares its live level atomic with a Python-free
threshold index. `FemtoLogAdapter::enabled()` reads that index without the GIL,
logger creation, or log emission. A registered normalized target uses its own
level; unknown valid targets use the default `INFO` level while the registry
can still admit them, and the root level once it cannot, because dispatch then
falls back to root; invalid targets use the root level or the default. Parent
levels are not inherited, and this early check does not inspect filters or
handlers.

The capacity question is answered by the same predicate `get_logger` refuses
on, so filtering and routing agree about whether a logger would be created.
That predicate takes the manager read lock, always after the threshold-index
lookup has released it.

When a logger enters the registry, manager code holds `MANAGER` before taking
the threshold-index write lock. The enablement lookup reads only the index.
Global `log::max_level()` refreshes after successful bridge installation,
logger level changes, staged configuration commits, and manager reset. Its
calculation includes the default level, so the facade macro prefilter cannot
hide a record accepted for an unknown target. A staged configuration commit
refreshes it while still holding `MANAGER`, so no dispatch can observe the new
index thresholds through a stale prefilter; the refresh takes only the index
lock, which the manager-then-index ordering already permits.

The root-level `FemtoLogAdapter`, install function, and logger lookup
re-exports are hidden and require both `log-compat` and `test-util`. Keep
benchmark-only access behind those feature gates. The Criterion benchmark can
be run with:

```shell
cargo bench --manifest-path rust_extension/Cargo.toml --no-default-features \
  --features log-compat --bench log_bridge
```

The Python-facing file, rotating-file, timed-rotating-file and stream handler
`flush()` and `close()` methods use PyO3's detached wait path. The `close()`
wrappers need only a shared Python-object borrow: they copy the shutdown state
into Rust-owned values, release that borrow, and then release the GIL while
they wait for the worker. Concurrent `close()` calls, or a `close()` beside a
detached `flush()`, do not contend for the handler. Every caller waits on the
same completion broadcast, so one closer does not return early merely because
another has already taken the shutdown state. The file family bounds each wait
to one second; the stream handler reuses its configurable `flush_timeout`
(default one second) for both. Preserve the handlers' existing timeouts and
idempotent-close behaviour when changing this boundary.

The stream handler applies that timeout in two places. `flush()` computes one
deadline and shares it between queueing the flush command and waiting for the
worker's acknowledgement, so a full queue cannot let a call take nearly twice
the configured limit. `close()` waits for the worker's completion signal, which
the worker raises after dropping its writer and formatter, so a successful
close means the worker body has ended, not that it drained the queue or
delivered every record: a panic still raises the signal with records queued,
and write errors are logged rather than returned. It does not mean the thread
has exited: thread-local destructors run after the worker body returns, and
joining would wait for them, so the closer drops the join handle and detaches
the thread instead. The worker reports its own panic through `catch_unwind`,
because no caller reads the join result. The regression tests release blocking
destructors through channels rather than sleeping, and the Python tests poll the
`test-util`-gated waiter counter so they release a gated formatter only after
`flush()` or `close()` has entered its wait.

`flush()` returns one boolean for every way it can fail, so it names the
failing stage and cause in a trace event instead: a closed handler, a command
the queue could not accept before the deadline, a queue disconnected while the
command was being queued, an acknowledgement that timed out, a disconnected
acknowledgement channel, and an acknowledgement reporting an I/O error each get
their own fixed message. The queue likewise traces a full or disconnected queue
when it drops a record. Every message is a fixed string with no record payload,
so the trace adds no per-record or per-target dimensions, and all of them are
`trace` level, so they are not formatted until an operator turns them on. The
wait durations and outcome counters that would support alerting remain the
bounded telemetry specified by issue #551, which this handler does not yet
export.

### HTTP transport-origin suppression

An HTTP handler delivers on a dedicated worker thread, and `ureq` reports its
own progress through the `log` and `tracing` facades as DEBUG records under
`ureq.unit`, `ureq.stream`, and `ureq.pool`. With either bridge installed those
records are ordinary records that a target route can send back to the handler
that produced them, so each successful delivery would manufacture the next one.
`rust_extension/src/http_handler/transport_origin.rs` is the guard that stops
the loop. It lives under the HTTP adapter, because the worker is its only
producer.

The module holds a thread-local `Cell<bool>` rather than process-wide state:
two HTTP handlers on different threads must not hide each other's ordinary
records, and a record emitted from an application thread stays deliverable
while some other thread is inside a request. Callers reach the state only
through three crate-private functions and one guard:

- `enter_http_transport()` marks the rest of the thread's work.
- `suspend_http_transport()` suspends marking for nested work.
- `capture_record_origin()` turns the current state into a `RecordOrigin`.
- `TransportScope` is the `#[must_use]` RAII guard both entries return. Its
  drop restores whatever state was current before the scope began, which also
  covers a panic unwinding through a suspended bridge call. Its
  `PhantomData<Rc<()>>` keeps it out of `Send` and `Sync`, because the state it
  restores belongs to one OS thread. `transport_origin_tests.rs` pins that with
  a compile-time assertion and with a property over arbitrary enter, suspend
  and drop sequences.

`spawn_worker` enters the transport scope once and holds the guard for the
whole worker lifetime, so agent construction, requests, retries, drain and
destruction all run marked. The bridges use the opposite ordering: each
captures the origin at entry, attaches it to the record, and then suspends the
scope for the rest of the call. A filter, formatter, or handler callback that
logs during dispatch therefore emits an application record instead of
inheriting the transport origin.

`RecordOrigin` (`rust_extension/src/log_record.rs`) is a crate-private
`Application`/`SelfDiagnostic` field on `FemtoLogRecord`. The variant names the
relationship — the logger reporting on its own work — rather than the component
that emits it, so the core record type carries no HTTP vocabulary and the
adapter keeps that vocabulary to itself. `Clone`, the handler-filter snapshot,
and parent propagation all preserve it, and no other consumer reads it, so it
never reaches the Python record view, formatted output or an HTTP payload.

The producer path only carries the value. The ordinary entry points attach
`RecordOrigin::Application`, and `log_prebuilt` accepts a record a bridge
assembled and returns it to the producer unchanged, so no core logger API names
an origin. A bridge therefore builds its record, attaches the origin it
captured at entry, and hands it over; nothing downstream rewrites it.

`FemtoHTTPHandler::handle` checks the origin after its closed-handler check and
returns `Ok(())` for a self-diagnostic record. The record is not queued, warned
about, nor counted as a drop: it never entered this handler's queue, so
reporting it as a loss would misdescribe a healthy worker. Non-HTTP handlers
still receive the record, which keeps independent diagnostics available.

Suppression is silent and has no counter or summary of its own. Most of the
records it discards are the transport library's own DEBUG output, so a
maintainer who needs to see them raises the `ureq` targets (`ureq.unit`,
`ureq.stream`, `ureq.pool`) to DEBUG and watches a non-HTTP sink. A record that
appears there and never arrives at the endpoint is evidence of suppression
rather than an exact count, because a filter or a failed delivery can also
account for an absence; comparing the two sinks is the route
`tests/http_transport_feedback_tests.rs` takes. A counter would need a
diagnostics surface this handler does not have. Its only existing signal is the
drop warner, and a suppressed record is deliberately not a drop; bounded
handler telemetry is tracked separately by #551.

The behavioural regression is `tests/http_transport_feedback_tests.rs`. Each
variant runs in a fresh child process under a watchdog against a local TCP
collector, enables real `ureq` DEBUG diagnostics, and requires delivery to stop
after the initiating records: two application requests arrive, the `ureq.unit`
diagnostic reaches a non-HTTP file sink, and a quiet window then passes with no
further request and no overflow. The child, its assertions, and the process
supervisor live in `tests/http_transport_feedback/`, where Cargo treats them as
modules rather than as separate test crates.

The marker is deliberately narrow. A record that Python code rebuilds from a
marked record is a new application record, records passed directly to the
three-argument `handle` method carry no marker, and socket handlers do not
suppress marked records.

### Handler shutdown signal, clock and condition-variable seams

`ShutdownSignal` (`rust_extension/src/shutdown_signal.rs`) is the second
channel a handler's worker can hear. A close sent through the command channel
queues behind the backlog it is meant to cancel, so it could not bound a
worker's retries or its blocking network calls. The signal carries no work,
only a [`Phase`], and the worker consults it between attempts and inside every
timeout.

The phases are `Running`, `Draining { deadline }` and `Aborted`. A drain's
first deadline is the binding one: `begin_shutdown` after it, or after an
abort, does nothing. `abort` is permanent. `remaining()` reports the budget
left in `Draining` and `None` while running (no budget applies) and after an
abort (none is left); callers that need to tell those apart ask
`is_shutting_down()` first. `clamp(configured)` is the query most call sites
want, because it already distinguishes them, and it returns `None` rather than
a zero duration: `std` and `ureq` both read a zero timeout as "no timeout",
which is the opposite of a spent budget.

Three seams exist so that behaviour can be tested at a chosen instant rather
than at whatever instant the host's scheduler supplies.

`monotonic_clock::MonotonicClock` (`rust_extension/src/monotonic_clock.rs`)
supplies "now" to the deadline queries. Production uses `SystemClock`, which is
`Instant::now`; tests construct a `ShutdownSignal::with_clock` around a
`ManualClock`, which moves only when told to. This is deliberately not
`rate_limited_warner::Clock`: that trait reports milliseconds since its own
construction and so cannot express an absolute `Instant` deadline. The stream
handler's flush deadline is a relative budget expressed in milliseconds, so it
does reuse that trait instead: the handler takes the clock from its rate
limiter, and a test that scripts it controls the deadline and the drop warnings
together. Each query samples the clock once, so the answer belongs to one
instant; a test that compared two separately-read queries would be comparing
two moments, which is the race the manual clock exists to remove. `wait` reads
both clocks on purpose: the deadline comparison uses the signal's clock so it
can never disagree with `is_aborted`, while the sleep bound is a real duration
handed to `Condvar::wait_timeout`, which has no seam. A manual clock that never
advanced would leave that bound constant and the loop unable to finish.

`sync::Condvar` is the platform seam behind the notification the workers park
on. The phase is mutated under its guard and `notify_all` runs while the guard
is still held, so no waiter can have released the lock without having parked.
`wait` re-reads the phase after each wakeup rather than assuming the wakeup was
a phase change, and it terminates on the real clock as described above.

The deadline is a bound on what a worker *starts*, not on what it has already
started: no `std` API cancels a blocking `connect`, a `write`, or a TLS
handshake. Nothing new begins once the deadline has passed or the phase is
aborted, and nothing started after a drain began outlasts the budget's
remainder. Worker termination is therefore bounded only loosely: by the budget
plus one call already in flight when the deadline fell, and not at all for the
two calls with no interruptible timeout, an already-started Unix-domain connect
and TCP hostname resolution. `close()` stays bounded regardless because it
detaches a worker that misses the deadline; worker termination itself carries
no such promise.

`discard_rest` in each worker's module is the record of what that bound costs.
Records still queued at the deadline are dropped and counted rather than sent,
and `warn_discarded` emits one rate-limited warning naming the count. The same
accounting covers the write path, where a frame that runs out of budget
mid-write reports how many of its bytes were sent, so a collector that is slow
can be told apart from one that has stopped reading.

Detaching is not draining to completion. `abandon_worker`
(`rust_extension/src/http_handler/handler.rs`, and the socket handler's
equivalent) must `abort` the signal and take the join handle, because the
detached thread is the only other holder of the signal: one left in `Draining`
would keep telling it that budget remained past the deadline it had already
reached. The worker then finishes its in-flight call, discards whatever is
still queued, and exits, with nothing left to join it, including the `Drop`
that may follow.

Tests covering these seams:

- `shutdown_signal_tests.rs` pins each transition and each query.
- `shutdown_signal_property_tests.rs` generates operation sequences over a
  `ManualClock` and asserts, after every operation, that `is_shutting_down`,
  `is_aborted`, `remaining` and `clamp` agree with the phase and the instant.
  It complements the Loom models under `tests/heavy/`, which explore thread
  interleavings rather than operation orders.
- `socket_handler/transport.rs` and `socket_handler/frame_write.rs` each carry
  boundary tables over the pure budget helpers, covering a deadline exactly
  reached, one nanosecond short, one long past, and a zero remainder.

Keep the helper functions taking the sampled instant as a parameter. A helper
that read the clock itself would make the sequence of budgets depend on how
long the previous call took, which is untestable and is the defect the seam
exists to prevent.

### Shared shutdown acknowledgement

`shutdown_ack::acknowledge_shutdowns` (`rust_extension/src/shutdown_ack.rs`) is
the acknowledgement step both network workers run as they stop. Its call sites
are `rust_extension/src/http_handler/worker.rs` and
`rust_extension/src/socket_handler/worker.rs`. Each worker holds two kinds of
shutdown sender: those queued on its command channel, and its own, which
`close()` sends and the worker holds for the final handshake. The helper drains
the queued senders first, then sends the worker's own acknowledgement if it
still has one, so a caller waiting on either acknowledgement cannot observe the
worker exit before the queued requesters have been answered.

A failed send means that the requester has already gone away, so the helper
ignores it and carries on. Nothing is reported back: a worker's exit path has
no recovery for a vanished sender, and stopping at the first failure would
leave later requesters waiting for an acknowledgement that never comes.

The helper lives in its own module because the two workers' stop paths are
otherwise separate, and this ordering is a shared contract rather than an
incidental similarity; the protocol concerns crossbeam channels, not either
transport. A change to the helper affects both handlers.

### Timed-rotation module boundary

`FemtoTimedRotatingFileHandler` lives in
`rust_extension/src/handlers/timed_rotating/handler.rs` and `mod.rs` re-exports
it, so the `timed_rotating::FemtoTimedRotatingFileHandler` path is unchanged;
code that reached it through `timed_rotating::core` must follow it. `core.rs`
keeps the rotation mechanics and `TimedRotationConfig`, which `mod.rs`
re-exports for the crate, and no longer defines the handler. Both files stay
under the 400-line module cap.

### Flush clock and observer seam

`rust_extension/src/logger/flush.rs` measures every flush through
`measure_flush_attempt(clock, observer, attempt)`. It reads the injected
`MonotonicClock` either side of one attempt, so the recorded outcome and
duration always describe the same flush. Neither the clock nor the observer is
ambient: the `FlushObserver` trait (`record(outcome, elapsed)`) and the clock
are parameters, and production selects them at the flush boundary.
`MetricsFlushObserver` reports to the process-wide flush counters, and with
`tracing-compat` the `TracingFlushObserver` around it records the outcome and
elapsed time on the `logger_flush` span before delegating. Tests drive the
production orchestration through `flush_handlers_measured` with a clock and
observer they own — including their own `FlushMetrics` — so an assertion cannot
be moved by another logger in the process.

`DrainOutcome` in `rust_extension/src/logger/dispatcher/shard/mod.rs` records
how a barrier wait ended: `Drained`, `QueueClosed` when the shard is gone for
good, and `TimedOut` when the worker is alive but did not answer the barrier
within `BARRIER_TIMEOUT`. A flush therefore reports which of the two failures
it hit rather than a bare false.

The seam exists because the alternative is untestable. A helper that sampled
the clock itself would make the measured duration depend on how long the
previous call took, and one that wrote to the process-wide counters directly
would share state with every logger in the process, so neither a unit test nor
a sibling logger could rely on what it read.

Python handler call plumbing — method lookup, the call itself, and the bounded
flush-failure categories — is in
`rust_extension/src/logger/py_handler/calls.rs`. The boundary around it is
split in two: `rust_extension/src/logger/py_handler/flush.rs` attaches to the
interpreter, runs the flush inside the `flush_registered_handler` span and
reports the outcome, while
`rust_extension/src/logger/py_handler/native_handler.rs` unwraps a built-in
handler and runs its flush with the GIL detached.

### Dependabot grouping for the extension build

The project is uv-managed (`uv.lock`, no requirements files), so the Python
entries of `.github/dependabot.yml` are `uv`, not `pip`. maturin (the `uv`
ecosystem) and PyO3 (the `cargo` ecosystem) are versioned independently yet
build one extension, so an update to either alone may not build against the
other. A multi-ecosystem group, `maturin-pyo3`, defined at the top of the file
with its own daily schedule and `open-pull-requests-limit` (Dependabot rejects
that key on an entry that joins a group), puts the available updates for the
pair into one pull request. When both have one, a single CI run builds them
together; a bump is not held back when only one of the two has an update, and a
group cannot choose compatible versions or rewrite a pinned literal, so the
maturin pins in the development group and the build workflows are still kept
aligned by `tests/test_maturin_build.py`.

An entry that joins a group runs only that group, over the dependencies its
`patterns` take: its own groups, ignore rules and every other dependency are
not processed (cuprum's first run of this arrangement showed it). So `uv` and
`cargo` each have two entries. The group entry, listed first, offers `maturin`
for `uv` and `pyo3` for `cargo` and carries no groups. The plain entry is the
stanza as it was, with its catch-all group, and gains an `ignore` of the same
name so the pair is handled once. The plain `uv` entry also ignores
`pytest-bdd`: it is pinned beside `pytest` in `pyproject.toml`, and a release
has to be checked against the pytest it supports before the pair moves, so it
is updated by hand.

`tests/test_dependabot_config_contract.py` holds the shape. Concordat's
`dependency-family-contracts` rule (DF-002) accepts a shared multi-ecosystem
group as one way to protect an independently versioned pair, reads the first
entry that processes each package
(<https://github.com/leynos/concordat/pull/334>,
<https://github.com/leynos/concordat/pull/338>), and accepts an `ignore` rule
for the pytest and pytest-bdd pair.

## Rust build standard

Development, test, lint and typecheck builds link with the `mold` linker on
Linux. This is a default in `.cargo/config.toml`, which Cargo discovers from
the repository root or from `rust_extension/`, so a bare `cargo build` and
`maturin develop` get it. The default compiler driver must be GCC 12.1 or
newer, or clang: older GCC drivers reject `-fuse-ld=mold` even when `mold` is
installed, and a host with one sets `-Clinker=clang`. `mold` ships for Linux
only, so the flag lives in a Linux-only target table, and macOS and Windows
keep their platform linker. On Linux, install `mold` before building: the
configuration names it, so a build without it fails at link time. The pin is
the stable `1.99.1`, so the nightly-only parallel frontend flag (`-Zthreads=8`)
is not used.

Every target that compiles Rust depends on `make check-build-tools`, which
reports a missing pinned toolchain or `mold` with the command that installs it.
`make install-build-tools` installs the toolchain from `rust-toolchain.toml`
and, on Linux, the `mold` package. The check is a capability probe: it installs
nothing, so it is safe to run before every gate. Installed tooling need not be
active in every job; release, coverage and Whitaker routes deliberately exclude
parts of the development standard, as described below.

Both scripts read `BUILD_TOOLS_OS` in place of `uname -s` when it is set, so
the installer and preflight contract tests can exercise the Linux and non-Linux
branches without leaving the host.

Before the first suite step, install the pinned toolchain and `mold` in every
hosted Linux job that runs a test suite step, or a Make goal that reaches the
suite: place an unconditional `setup-rust` step with `install-mold: 'true'`
ahead of that step. A late, conditional, soft-failing or `mold`-less install
does not satisfy the rule, because the suite's Make goals reach
`check-build-tools`. The contract is derived from Make prerequisite
reachability in
[`tests/test_workflow_tool_provisioning_contract.py`](../tests/test_workflow_tool_provisioning_contract.py),
so a new suite job is covered without being named there.

The build-standard tests are split by what each one pins. The capability
scenarios that judge a host live in
[`tests/test_build_tools_preflight_script_contract.py`](../tests/test_build_tools_preflight_script_contract.py);
the rustup probes' own mechanics, such as running from `/` and refusing to
auto-install, in
[`tests/test_build_tools_probe_contract.py`](../tests/test_build_tools_probe_contract.py);
and the Makefile's dependency, ordering and substitutability rules in
[`tests/test_build_tools_contract.py`](../tests/test_build_tools_contract.py).
The installer's own contract, which runs every case through both
`bash scripts/install-build-tools.sh` and `make install-build-tools`, lives in
[`tests/test_build_tools_installer_script_contract.py`](../tests/test_build_tools_installer_script_contract.py).
Their machinery has modules of its own:
[`tests/build_tools_support.py`](../tests/build_tools_support.py) builds the
fake executables,
[`tests/build_tools_script_support.py`](../tests/build_tools_script_support.py)
varies the rustup stub's answers, and
[`tests/build_tools_probe_support.py`](../tests/build_tools_probe_support.py)
runs the script against them.

The workflow readers divide the same way.
[`tests/workflow_document_helpers.py`](../tests/workflow_document_helpers.py)
discovers and parses the documents, checks that every locally reusable workflow
a job calls exists, and offers the job and step accessors.
[`tests/workflow_command_helpers.py`](../tests/workflow_command_helpers.py)
reads a `run` body's shell text, with
[`tests/workflow_shell_helpers.py`](../tests/workflow_shell_helpers.py) holding
the quoting scan both readers share;
[`tests/workflow_heredoc_helpers.py`](../tests/workflow_heredoc_helpers.py)
finds the operators that scan leaves unquoted and skips their bodies;
[`tests/workflow_runner_helpers.py`](../tests/workflow_runner_helpers.py)
classifies runners; and
[`tests/workflow_contract_helpers.py`](../tests/workflow_contract_helpers.py)
states the provisioning rules and consumes the other three.
[`tests/test_workflow_contract_properties.py`](../tests/test_workflow_contract_properties.py)
is the Hypothesis suite that checks those readers against independent models,
and [`tests/make_contract_helpers.py`](../tests/make_contract_helpers.py)
supplies `prerequisite_map()`, the Makeutil-backed prerequisite map.

The readers take their outside inputs from the caller rather than reaching for
them: `depends_on_preflight` accepts a prerequisite map and `workflow_jobs`
accepts a `reusable_exists` lookup, so neither parses the Makefile nor reads
the repository itself.

The preflight runs `rustup toolchain list` and `rustup show` from `/`, so the
repository's `rust-toolchain.toml` cannot provision or redirect the pinned
channel. It matches the installed name as `<channel>-<host>`, and accepts a bare
`<channel>` only when `rustup show` reports no host. A probe that fails is
named in the report rather than described as an absent toolchain, so an
operator can tell a broken rustup from a missing installation.

Every direct Cargo command line in the Makefile uses the `CARGO` variable,
which defaults to `cargo`. Overriding it runs the same recipe with a different
executable, so `make test CARGO=/opt/rust/bin/cargo` exercises another Cargo
installation without editing the Makefile. The `fmt`, `check-fmt`, `clean`,
`release`, `test` and `lint-rust-test-docs` targets carry such command lines.
`build` drives `maturin` instead, and the Clippy and environment-policy lane
drivers (`lint-rust-clippy`, `lint-env-policy`) select Cargo themselves, so
`CARGO` does not redirect them.

An assigned `RUSTFLAGS` replaces every `rustflags` source, so the file has no
`[build] rustflags` and each build that assigns `RUSTFLAGS` decides about the
linker itself:

- CI's `setup-rust` exports
  `rustflags: '-D warnings -Clink-arg=-fuse-ld=mold'`, restating the linker
  flag the export would otherwise displace, and installs `mold`
  (`install-mold: 'true'`). The coverage step assigns a warning-only
  `RUSTFLAGS` again, because the instrumented measurement excludes the
  development linker. The compile-time UI tests (trybuild) build a nested crate
  under `target/` that finds `.cargo/config.toml`; with the flag restated, that
  nested build keeps the same linker as the surrounding job.
- The scheduled `heavy-tests` workflow installs `mold` through `setup-rust`
  (`install-mold: 'true'`, `rustflags: ''`) and its three Loom steps assign
  `RUSTFLAGS="--cfg loom -C link-arg=-fuse-ld=mold"`.
- Release wheel builds run in manylinux containers that carry no `mold`, so
  each Linux `maturin-action` step assigns an empty `RUSTFLAGS`, which
  displaces the Linux table. maturin-action forwards `RUST*` variables into the
  container, so the wheel links with the container's own toolchain.
- `make lint-rust` runs Whitaker as
  `RUSTFLAGS="-D warnings" … $(WHITAKER) --all --manifest-path …`. Whitaker
  runs under the toolchain its installer manages, so that literal `RUSTFLAGS`
  is the isolation boundary: it takes none of the development standard (no
  `mold` flag and no inherited `RUSTFLAGS`).

### Locked dependency fetch before tests

A test that starts its own Cargo build (the trybuild compile-time UI tests, for
one) must not resolve dependencies while it holds the shared package-cache
lock, or a cold cache stalls every other Cargo process on the host. Every test
entry point therefore fetches first, and `cargo fetch --locked` must succeed
before any `cargo test` starts: the `test` recipe of the Makefile (whose
prerequisites only build, lint and unit-test the Loom runner script against a
mocked Cargo) begins with
`cargo fetch --locked --manifest-path $(RUST_MANIFEST)` on its own line (so a
failed fetch stops the recipe), and the CI and heavy-test workflows each run an
unconditional `Fetch locked dependencies` step with
`--manifest-path rust_extension/Cargo.toml` before their first test step. Keep
a new test-running step after that step. `tests/test_locked_fetch_contract.py`
parses the Makefile and both workflows and fails on a missing, unlocked,
mis-pointed, skippable, non-failing or late fetch (Concordat's
`nested-cargo-offline-tests` rule, NC-008).

### Development build footprint

The development profile sets `incremental = false` and `debug = 0` in
`rust_extension/Cargo.toml`, so a dev build keeps no incremental state or debug
information. To debug, set `CARGO_PROFILE_DEV_DEBUG=true`; to rebuild
incrementally, set `CARGO_INCREMENTAL=1` (unset any `sccache` wrapper first,
which refuses incremental builds). The pin is stable, so the native Cargo
metadata key (`[unstable] embed-metadata`) does not apply and the configuration
must not carry it.

`tests/test_build_standard_contract.py` holds all of this: the Linux-only
`mold` table, the profile settings, the Loom steps, the CI restatement and
coverage exclusion, the preflight wiring, the release-wheel override and the
Whitaker boundary. It proves the defaults and both overrides at the Cargo
boundary by building a one-file crate under the repository's own configuration.
That fixture also drops the wrapper variables from its environment and passes
empty `build.rustc-wrapper` and `build.rustc-workspace-wrapper` values on the
Cargo command line. A host that configures a compiler wrapper in its own Cargo
configuration would otherwise queue the one-file build behind that wrapper, and
the measurement would describe the host instead of this repository. The fixture
also builds with a private `CARGO_HOME`, so it reads no host configuration at
all.

### Cranelift

Exception: Cranelift is not the development-profile backend. Cranelift requires
a nightly toolchain, and this repository pins the stable `1.99.1` (recorded
2026-10-31, with no measurement taken on it). Revisit if the repository moves
to a nightly pin.

## Frame filtering and exception-graph limits

The Python `filter_frames()` binding owns payload classification, frame
filtering, and exception-graph safety. Exception payloads are traversed by the
`ExceptionGraphTraversal` work stack in the Rust frame-filtering module. Each
`Enter` work item checks active-path dictionary identity, exception depth, and
the root-inclusive node budget before filtering and attaching a shallow copy.
It then schedules `cause` and `context` edges and deferred `FilterGroup` work.
The LIFO stack visits cause, context, and exception-group members in their
established order. When `FilterGroup` work is popped, it validates the
`exceptions` list and checks its full width against the remaining node budget
before allocating the output list or scheduling members.

Cycle detection tracks only dictionaries on the active ancestor path, so shared
acyclic subgraphs remain valid. The root has graph depth zero;
`max_exception_depth` defaults to 1000 and `max_exception_nodes` defaults to
10,000 occurrences, including the root. Both graph limits are ignored for stack
payloads. The frame-list `max_depth` still limits retained frames in each
payload only.

Malformed frames and exception edges raise `TypeError`; cycles, exceeded graph
limits, and a zero exception node budget raise `ValueError`; negative or
oversized integer limits raise `OverflowError`. The binding returns these
Python exceptions and does not emit logs or metrics. Applications that need
operational counts can instrument calls at their own boundary.

See the [frame-filtering design](frame-filtering-design.md) for the public
contract and diagrams, and the [Python type stub][filter-frames-stub] for the
public signature. The [Rust graph tests][rust-graph-tests],
[Python graph tests][python-graph-tests], and
[Python graph property tests][python-graph-property-tests] cover the traversal
boundaries.

[filter-frames-stub]: ../femtologging/_femtologging_rs.pyi
[rust-graph-tests]: ../rust_extension/src/frame_filter_py_graph_tests.rs
[python-graph-tests]: ../tests/frame_filter/test_exception_graph_limits.py
[python-graph-property-tests]: ../tests/frame_filter/test_exception_graph_properties.py

## Publishing a release

The first release has not yet been published. Before creating its tag,
configure the GitHub environment and PyPI Trusted Publisher described below.
Bump the version in both `pyproject.toml` and `rust_extension/Cargo.toml` to
the same exact string, then commit and push those changes. The Rust extension
crate is `publish = false`; maturin includes it in the Python distribution, and
it is never published with `cargo publish`.

Push a tag whose version matches both manifests, for example `v0.2.0-beta1`. The
`release.yml` workflow accepts `v*.*.*` tags, compares the tag with the Cargo
package version and the exact Python project version, and marks pre-releases
from the Python version. The versions must match without the tag's leading `v`.

The workflow then proceeds through these jobs:

- `build` calls `build-wheels.yml`. It builds per-interpreter CPython 3.12,
  3.13 and 3.14 wheels for Linux x86_64 and aarch64, macOS x86_64 and arm64,
  and Windows x86_64, plus exactly one source distribution. Each building job
  installs and imports the wheels it produced before uploading them, so a wheel
  that cannot install never becomes an artefact. The build workflow also runs
  in pull-request CI; its verification job fails if the sdist is absent or
  duplicated and installs a wheel and a rebuilt sdist.
- `attest` collects every wheel and the single sdist, attests those files with
  build provenance, and passes the files and Sigstore bundle to later jobs as
  one artefact.
- `draft-release` creates or reuses the tag's GitHub Release as a draft and
  snapshots any existing release assets.
- `publish-pypi` runs in the `pypi` environment with Trusted Publishing. It
  checks the PyPI Simple index, stages names PyPI does not already hold, and
  uploads only those files with `skip-existing` enabled.
- `publish-release` reconciles the GitHub Release with PyPI, attaches only
  provenance bundles whose subjects match the published bytes, checks that both
  destinations have the same SHA-256 digest for every distribution file and
  that every file is attested, then publishes the draft. Pre-release status
  follows the checked Python version. See
  [Reconciling release assets locally](#reconciling-release-assets-locally) to
  run the same reconciliation by hand.

Re-running the same tag is safe. Runs for one tag queue rather than cancel each
other, and the workflow never overwrites an existing filename. For each name,
existing PyPI bytes take precedence, then existing GitHub Release bytes, then
the current build. Each destination receives only missing names, so a complete
rerun has nothing new to upload. If the destinations already hold different
bytes for the same name, digest verification fails and the release remains a
draft. PyPI files cannot be deleted; resolve the conflict by removing the
inconsistent asset from the GitHub Release, then rerun the tag.

Attestations follow the published bytes rather than the current build. A bundle
that attests a rebuilt file whose canonical bytes came from PyPI is skipped,
and verification requires every published file to be covered by a bundle
attached to the release, either by this run or by an earlier one. A
carried-over file with no matching attestation fails verification and leaves
the release as a draft.

The check is structural rather than cryptographic: it compares in-toto subject
digests with the published bytes and does not validate the Sigstore signature
or certificate, nor does it run `gh attestation verify`. An actor able to write
release assets already controls those bytes, so structural coverage is the
intended guarantee; cryptographic provenance verification would be a change to
the release contract.

### Reconciling release assets locally

`scripts.release_assets_cli` is the entry point the `release.yml` jobs use after
`build` and `attest`. It has no network access: workflow steps fetch the PyPI
Simple index and the GitHub Release asset list with `gh` or `curl`, then pass
the saved files to the CLI. Run a command from the repository root as
`python3 -m scripts.release_assets_cli <command>`.

Every command reads the build directory through `--publish`. `--index` is
required by every command except `carryover`, and `--assets` is always
required; both name snapshots that the caller has already downloaded.

| Command         | Required paths                                                                 | Optional paths       | Effect                                                                                                                                                                                                                                                |
| --------------- | ------------------------------------------------------------------------------ | -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `carryover`     | `--publish`, `--assets`                                                        | –                    | Prints names held by both the build and the release, one per line, for a caller that downloads them.                                                                                                                                                  |
| `stage-pypi`    | `--publish`, `--index`, `--assets`, `--github-files`, `--github-output`        | –                    | Removes names already on PyPI from `--publish` and replaces a rebuilt name whose canonical bytes are an existing release asset with the digest-checked file from `--github-files`. Writes `remaining=true` or `remaining=false` to `--github-output`. |
| `stage-github`  | `--publish`, `--index`, `--assets`, `--bundles`, `--output`, `--github-output` | `--github-files`     | Stages missing release assets into `--output` and copies bundles whose subjects match them. Prints `name<TAB>url` for files whose canonical bytes are on PyPI, and writes `pending=true` or `pending=false` to `--github-output`.                     |
| `check-fetched` | `--publish`, `--index`, `--assets`, `--output`                                 | –                    | Compares each file in `--output` downloaded from PyPI with its index digest and fails on a mismatch.                                                                                                                                                  |
| `verify`        | `--publish`, `--index`, `--assets`, `--bundles`                                | `--existing-bundles` | Fails unless both destinations hold the same SHA-256 digest for every distribution file and every one is attested. Prints a success line when they are.                                                                                               |

Commands return `0` on success and `1` on a reconciliation error, which is
printed to standard error as `::error title=release-assets::<reason>`.
`--existing-bundles` names the bundles an earlier run attached to the release;
without it, `verify` can only account for the bundles staged in the current run.

The commands implement the precedence described above rather than re-deriving
it. `stage-pypi` omits names PyPI already holds, `stage-github` prefers PyPI
bytes and prints the URLs its caller must fetch, `check-fetched` confirms those
downloads match the index, and `verify` compares both destinations. See
[Publishing a release](#publishing-a-release) for the release contract these
checks enforce.

### Required publishing settings

Configure these settings manually before the first tag:

- In GitHub, create the `pypi` environment and restrict deployments to tags
  matching `v*.*.*`.
- In PyPI, add a Trusted Publisher for project `femtologging`, owner `leynos`,
  repository `femtologging`, workflow `release.yml`, and environment `pypi`.

The PyPI project and publisher are not registered by this repository change.
The first release cannot publish until the publisher and GitHub environment
exist. See [Maturin build features](#maturin-build-features) for the production
and test feature sets.

## Type checking and heavy tests

`make typecheck` depends on `make build`, which creates `./.venv`, installs the
development dependencies, and installs the editable Rust extension there. The
target then invokes:

```shell
ty check --python ./.venv
```

The `--python` option points `ty` at the project environment containing the
extension and its dependencies; the same interpreter is recorded in
`[tool.ty.environment]` in `pyproject.toml`, and the Makefile names it
explicitly because the pinned local `ty` version does not reliably apply the
equivalent project settings.

The long-running Rust integration suite is the Cargo `heavy` test target,
rooted at `rust_extension/tests/heavy/main.rs`. Its property-based tests are
marked `#[ignore]` because each generated case starts a handler worker. Run the
target explicitly when investigating it:

```shell
cargo test --manifest-path rust_extension/Cargo.toml --no-default-features \
  --test heavy -- --ignored
```

The scheduled `heavy-tests` workflow runs ignored tests across its feature
lanes. Loom model test functions are compiled and registered only when Cargo is
invoked with `--cfg loom`; the ordinary heavy run does not compile or run them.

### Loom models

Eleven Loom models in `rust_extension/tests/heavy/` check the handler and
logger workers: delivery through the stream handler's queue, routing through
the logger to every attached handler exactly once, concurrent handler
attachment, concurrent writes and flushes through the file handler, the shared
worker-completion signal that concurrent closers wait on, and flush ordering.
Two of those models drive the shutdown guard the stream worker really uses, so
they require a closer that observes completion to see the worker's writer and
formatter already dropped, both when the body ends normally and when it
unwinds. The flush model varies the number of records queued ahead of the
barrier, including none, and whether the handler reports failure; it asserts
that the drain barrier is acknowledged only after every record queued ahead of
it has been handled, and that handlers are flushed only after that
acknowledgement. Handler models run the handler worker code with Loom's
synchronization primitives. Logger models use a Loom-specific logger-owned
shard rather than the process-wide production `Dispatcher`; dropping the logger
sends `Job::Stop` and joins that worker during model cleanup. Every worker
spawns, queues and locks through the private seam in
`rust_extension/src/sync.rs`, which is `std::thread`, `crossbeam_channel` and
`parking_lot` in an ordinary build and Loom's thread, locks and a small bounded
channel under `--cfg loom`. Loom arrives only as a `cfg(loom)` target
dependency. The Python-specific paths — the GIL-detached wait and delivery, and
the manager-lock release before a bridge flush — are outside the
`--no-default-features` Loom lane; the Python regression tests cover them
instead.

The completion models schedule registration against completion, a repeated
completion, and a waiter that arrives after the worker has stopped. They use the
`test-util` `is_completed` probe for the unfinished state, because Loom has no
clock and its timed waits never expire; a model that waited on an unfinished
completion would hang instead of failing.

The workflow has two Loom steps, and the split is deliberate. "Compile Loom
heavy tests" passes `--no-run` and proves the models build. "Run Loom models"
executes them under `scripts/run_loom_models.py`:

```shell
RUSTFLAGS="--cfg loom -C link-arg=-fuse-ld=mold" LOOM_MAX_PREEMPTIONS=3 uv run --script \
  scripts/run_loom_models.py --expected rust_extension/tests/heavy/loom-models.txt -- \
  cargo test --release --manifest-path rust_extension/Cargo.toml \
  --no-default-features --test heavy loom_ -- --include-ignored
```

The checker fails the step unless every model named in `loom-models.txt` is
reported as passed and nothing else is. `cargo test` alone passes with zero
tests when `--cfg loom` is dropped, when the filter matches nothing, when
`--ignored` replaces `--include-ignored`, and when `--no-run` is restored; the
checker fails each. `make loom-runner-test` tests it. A scheduled failure is
not left silent: the workflow's `notify-failure` job comments on, or opens, the
`heavy-tests nightly failure` issue with the run URL, commit and run date.
`tests/test_heavy_workflow_contract.py`, with its rules in
`tests/heavy_workflow_contract_helpers.py` and its document accessors in
`tests/heavy_workflow_document.py`, pins that the job installs from `uv.lock`
before building, keeps the Loom step last, and runs only for a scheduled
failure. Every rule is also applied to a deliberately broken copy of the
workflow, so a check that reports nothing cannot pass by accident.

Both steps build into `target/loom` under their own cache key, which names the
toolchain, the lockfile, the feature selection and the flags, apart from the
ordinary build. The build is `--release`, as Loom recommends: the topology
models take about eight minutes optimized and eleven not.

What the models establish is bounded. Loom explores every interleaving within
`LOOM_MAX_PREEMPTIONS` for the threads each model starts, which is a strong
statement about a bounded space and not a proof for all executions. Within
that, some operations are outside the model by design:

- Loom has no clock, so the timed waits, `recv_timeout` and `send_timeout`,
  wait without a bound. A worker that can fail to answer hangs the model
  instead of failing it, and the step's timeout turns that hang into a failure;
- the production shard worker blocks on `recv()` instead of polling an idle
  timeout. Loom models stop their owned worker with `Job::Stop`; no model
  treats an empty queue as shutdown;
- the handler list is shared through `std::sync::Arc`, whose reference counts
  Loom does not schedule;
- the topology models attach a synchronous test handler rather than stream
  handlers, because Loom models at most five threads. The stream handler's own
  queue is covered by `loom_stream_push_delivery`;
- the file model writes into a Loom buffer, not a file. Real file output is
  covered by the ordinary file-handler integration tests.

Under `cfg(loom)`, `FemtoLogger`'s `Drop` joins its worker directly rather than
through `Python::attach`, and the logger and file-worker unit tests, which
drive the workers with real threads, build in ordinary configurations only.
`docs/execplans/issue-470-execute-loom-models.md` records each decision and the
mutations each model rejects.

The execution step is the **last** step in the job, so a model failure never
stops the heavy suite, the Clippy lanes and `pytest` from reporting.

`tests/test_loom_lane_contract.py` asserts that the execution step runs
`cargo test` under the checker with the model list, sets `--cfg loom` and a
positive `LOOM_MAX_PREEMPTIONS`, selects every model, carries no `--no-run`, is
bounded and runs last; that the compile step does carry `--no-run`; and that
the model list names exactly the models the heavy target defines.

## Logger cardinality and shared dispatch

The accepted design for logger names and dispatch is recorded in
[ADR 0007](adrs/adr-0007-bounded-logger-cardinality-and-shared-dispatch.md).
Names are validated before registry lookup: no empty dotted segments, at most
255 UTF-8 bytes and 32 segments, and no control characters or Unicode line and
paragraph separators. The process-wide registry keeps at most 10,000 named
loggers; `root` is exempt, and a refused new name or parent chain is rejected
without evicting or partially inserting existing logger state.

The manager starts its process-wide dispatcher lazily when the first logger is
constructed. It starts between one and four shards, clamped from the machine's
available parallelism, then assigns each logger a shard round-robin. Each shard
has one named worker and a queue and permit pool of `1,024 × shard count`
records. Each logger has its own in-flight budget, capped at 1,024 and below
the shard depth. Queue, shard-permit, and per-logger-budget refusals return the
record to the producer; the logger increments its drop counter and emits a
rate-limited warning without waiting. One logger's records remain FIFO on its
shard, while records from different loggers on that shard share a worker and
can delay one another.

`flush_handlers()` queues a barrier only when that logger has work in flight.
The barrier drains records ahead of it on the same shard; an idle logger does
not consume a queue slot or wait for unrelated work. The Python wrapper
releases the GIL while the worker acknowledges the barrier and attached
handlers flush, so Python handlers can run. That includes a built-in handler
flushed through its native implementation: a Python formatter on its worker
thread needs the same GIL the flush would otherwise hold while waiting for the
acknowledgement. Enqueueing the flush marker and waiting for its
acknowledgement share a two-second timeout. Flushing attached handlers can take
additional time. The method returns `True` only when the worker acknowledges
the marker in time and every handler flush succeeds; it returns `False` if the
marker cannot be enqueued or acknowledged in time, or a handler flush fails.

The Rust `log::Log::flush` bridge snapshots logger references under the
`MANAGER` read guard, releases the guard, then detaches each logger's blocking
flush. This avoids a lock cycle: a Python manager writer can hold the GIL while
waiting for the write lock, while the detached flush waits for a Python
callback to acquire the GIL. The bridge ignores each logger's boolean result
because `log::Log::flush` returns `()`.

Builds with `python,test-util` also expose `wait_for_flush_entry_for_test()`,
`drain_flush_signals_for_test()`, and `cancel_flush_wait_for_test()` for
bounded regression-test coordination. Four hooks witness the GIL-release tests:
`note_python_thread_ran_for_test()` counts the calling thread as an observation
only while a detached wait is in flight and reports whether it counted,
`gil_release_observations_for_test()` reads that count,
`reset_gil_release_observations_for_test()` clears it, and
`wait_for_wait_in_flight_for_test()` waits until a wait has detached. Normal
`python` builds omit every one of these hooks, and
`femtologging/_femtologging_rs.pyi` declares them for type checkers.

Dropping a logger follows the same per-logger in-flight check. The pool itself
lives for the process lifetime. If the operating system refuses to start a
worker, construction returns `DispatcherError` in Rust or `RuntimeError`
through Python. The pool is not marked failed, so a later construction retries.

The production pool is outside the Loom model. Each Loom run owns and stops its
own shard so worker threads do not survive between model iterations; the
topology tests exercise routing and handler isolation with a smaller
`LOOM_RECORDS_PER_LOGGER` depth. The primitives behind both workers live in
`rust_extension/src/sync.rs`: production `spawn_named` returns the operating
system's spawn error and gives shard workers recognizable names, while Loom's
implementation keeps the same signature without process thread names. Its timed
send and receive operations keep their normal API shape, but Loom has no clock,
so the heavy lane's outer timeout bounds waits that would otherwise remain
blocked. The shard worker uses blocking `recv()` and has no idle polling tick.

## Releasing the GIL at the Python boundary

These file handlers let Python callers wait directly on a Rust channel, so
their bindings give the interpreter lock back for the duration of those waits.
The `flush()` and `close()` boundary is described in
[Python-facing `flush()` and `close()` waits](#python-facing-flush-and-close-waits);
this section covers `handle()`, whose wait is the unbounded one, and the
mechanics all three operations share. `FemtoFileHandler`'s `#[pymethods]`
block, and the rotating and timed-rotating wrappers that delegate to it, take a
`py: Python<'_>` token and use `py.detach(...)` around the operation.
`handle()` waits on channel capacity, unbounded under `OverflowPolicy::Block`
(`Sender::send`) and bounded by the deadline under `OverflowPolicy::Timeout`
(`send_timeout`). Other Python-exposed handler families also have operations
that may block. The stream, socket and HTTP bindings detach both their
`flush()` and their `close()` waits. The socket and HTTP `close()` methods take
an exclusive PyO3 borrow (`&mut self`) for the whole wait, so a second
`close()`, or a `close()` overlapping a detached `flush()`, is refused by the
borrow check rather than queued; the file and stream families use a shared
borrow and allow those overlaps.

The native-handler flush and the socket and HTTP `flush()` and `close()` waits
go through `rust_extension/src/py_detach.rs`. Its `pub(crate)` helper
`detach_for_wait(py, wait)` is the single place those bindings give the lock
back, so a new binding of that kind calls it rather than `py.detach` directly:
one audited boundary cannot be forgotten by a wait added later, and the
test-only in-flight record is entered in exactly one place.
`rust_extension/src/socket_handler/handler.rs` and
`rust_extension/src/http_handler/handler.rs` are the call sites to copy. The
helper enters the record while the GIL is still held and drops it after
`Python::detach` returns, which is what lets the GIL-release tests tell a
detached wait from one that merely finished quickly; production builds compile
the record out and leave `Python::detach` alone. The file, stream and rotating
families are the remaining exception: they run a direct `py.detach` inside
`WaitScope`, whose production gauge measures a different span.

`py.detach` requires its closure and return type to implement `Ungil`. On
stable Rust, PyO3 approximates that requirement through `Send`, so the bound
rules out a closure that captured a `Python<'_>` token, but it is an
approximation rather than a complete prohibition: a detached closure must still
be audited to ensure it does not reach Python-bound values or call Python APIs
without reattaching first. The compile-pass fixture in
`rust_extension/tests/ui/pass/pyo3_detach_ungil.rs` checks the accepted
pattern; it does not prove that every forbidden capture is rejected. The split
below is therefore deliberate, and the fixture guards its shape:

- argument parsing, `FemtoLevel::parse_py`, and record construction run
  **before** the detach, because they can raise;
- only the enqueue, the acknowledgement wait and the join run **inside** it;
- `PyRuntimeError`/`PyValueError` construction runs **after** it returns, on the
  GIL-owning side.

Detaching does not branch on the overflow policy. `OverflowPolicy::Drop` uses
`try_send` and never waits, but one uniform boundary is easier to audit than a
policy-shaped one, and it keeps a future change to the drop path from silently
reintroducing a wait on the locked side.

Because `--cfg loom` builds `--no-default-features`, the `#[pymethods]` blocks
still compile in that lane: PyO3 is an unconditional dependency, and the
`python` feature only governs extension-module linking. Detaching here needs no
`cfg(not(loom))` guard, unlike the logger worker's join: these methods receive
their `Python<'_>` token from the caller and never attach, so a Loom model that
somehow reached them would fail on the borrow or the token rather than
initializing an interpreter on a Loom coroutine's small stack. The heavy lane's
file model drives the Rust API (`with_writer_for_test`) and never reaches the
bindings at all.

### Testing the boundary

Neither the Rust tests nor the Loom models can see this behaviour. The Rust
tests call `FemtoHandlerTrait::handle` directly and never hold a GIL; the Loom
models replace the channel and the thread with Loom's equivalents, so there is
no interpreter to hold the lock in the first place. A Loom model that did drive
the bindings would model nothing it is meant to explore.

The evidence therefore comes from Python, in
`tests/test_file_handler_gil_release.py`, which runs each scenario in its own
subprocess. Isolation is the point: a binding that never releases the lock
deadlocks rather than failing, and an in-process test would hang the suite
instead of reporting. The parent applies a timeout and reports an expiry as a
probable GIL deadlock.

Each child parks the worker on a blocking formatter behind a capacity-1 queue,
makes the blocking call from its own thread, and only then starts a watchdog
thread that records a timestamp every few milliseconds. Starting the watchdog
after a settle period — and after asserting that the blocked thread is still
alive — is what makes the verdict sound: the call under test cannot return
while its queue is full or its worker is parked, so a call still running at
that point is inside the binding, and no record the watchdog makes can precede
its entry. The verdict then requires at least five records inside the call's
window and a window of at least half a second:

- a released wait of a second or more yields hundreds of records;
- a wait that holds the GIL yields none, because a single spin of the
  watchdog's loop is milliseconds of pure bytecode and the calling thread never
  releases the lock for that long.

The window check matters for the bounded cases. A `"timeout:N"` or `close()`
wait ends on its own deadline, so a call that returned early would otherwise
collect records trivially, and a bare ordering check would pass for the defect.

The subprocess scenarios pin one interleaving each, so they cannot enumerate
the order of `handle()`, `flush()`, and `close()` calls.
`tests/test_file_handler_lifecycle_state_machine.py` covers that gap with a
Hypothesis `RuleBasedStateMachine` that generates bounded call sequences for
all three builders across queue capacities and overflow policies, and asserts
the shutdown invariants after each step. It runs in-process and does not
control scheduling, so it complements the subprocess evidence rather than
replacing it: the GIL verdict still comes from the child interpreters.

## Producer/worker context propagation

The logger's producer path snapshots the configured handler list while holding
the handler read lock. That same snapshot is stored in each `QueuedRecord`,
along with the record and, when needed, a Python `contextvars` context. The
context decision must be derived from this snapshot, not from a separate
logger-wide flag: handler mutation may otherwise make the decision disagree
with the handlers that will actually receive the record.

`FemtoHandlerTrait::is_python_backed()` identifies handlers that can invoke
Python. When the queued snapshot contains one of these handlers, the producer
asks its injected `ContextSnapshotProvider` for the caller's context and stores
the result in `QueuedRecord::context`. The default
`ContextVarsSnapshotProvider` calls `contextvars.copy_context()`. Pairing this
snapshot with the cloned handler list keeps capture aligned with the handlers
that will receive the record. An absent context means that the snapshot
contains no Python-backed handler; it does not represent a failed capture.

Each capture failure increments a separate context-capture failure counter;
warning emission is rate-limited. These failures do not increment the
queue-full drop counter. The failure is handled at the producer command
boundary rather than being silently converted into an absent context.

The logger worker passes the queued context by reference to
`FemtoHandlerTrait::handle_with_context()` for every handler in the snapshot.
The default trait implementation delegates directly to `handle()` for native
Rust handlers. Python-backed handlers must report contextual dispatch through
`provides_context_dispatch()` and implement `handle_with_context()`; the
capability method must return `true` only when the implementation invokes
Python inside the supplied context. `FemtoLogger::add_handler()` returns
`HandlerError::MissingContextDispatch` if a Python-backed handler does not
declare this support. The default `handle_with_context()` also fails closed for
Python-backed handlers. `PyHandler` copies the captured context for each
callback and runs either `handle_record(record_dict)` or the legacy
`handle(logger, level, message)` call through `Context.run()`. Python filters,
formatters and other callbacks that execute synchronously within those calls
use the captured producer context, without allowing one Python handler to
modify another's view of it. Work deferred to another thread or process needs
separate context propagation; non-Python handlers retain their existing path.

For Python-backed handlers, the worker invokes
`StdlibHandlerAdapter.handle_record()` through `Context.run()` before that
method calls the wrapped handler's `handle()`. A direct call to
`handle_record()` does not establish that context. With base `Handler.handle()`
behaviour, the filter chain and `emit()` invocation run synchronously in the
captured producer context, as does formatting performed during that call. A
handler that defers formatting or emission to another thread or process needs
separate context propagation.

The HTTP transport origin travels through the same path at no extra cost. The
origin is a field on the queued `FemtoLogRecord` itself, so the handler-filter
snapshot, the `QueuedRecord`, parent propagation, and each handler's own clone
all carry it without a second thread-local read. Nothing after bridge entry
consults thread state to recover it, which is why a record marked there stays
marked as far as the HTTP handler's admission check.

### Standard-library handler adapter

The private `femtologging._adapter_record` module owns the adapter record
schema and conversion to `logging.LogRecord`. `femtologging.adapter` remains
the public import site and re-exports those record types. `FemtoLogger` applies
its logger-level threshold before dispatch. Once the worker dispatches a record
to the adapter, `handle_record()` calls `_make_log_record()`, compares the
mapped level with the wrapped handler's current `level`, then calls
`handler.handle()` exactly once when the record meets that threshold. This
handler-level comparison happens before the handler filter chain. With base
`Handler.handle()` behaviour, accepted records follow its existing filter and
lock path; a subclass overriding `handle()` owns its internal dispatch path.

With the `tracing-compat` feature enabled, trace events mark producer context
capture, handler-filter-chain evaluation, queue dispatch, worker handler
dispatch, and the Python-facing handler waits described below. Filter-chain
events report `accepted`, `rejected`, or `callback_failed`. If all attached
handler filters reject a record, the producer emits `producer_handler_dispatch`
with `handler_kind=none` and `dispatch_outcome=no_handlers_accepted`. Events
use bounded operation, handler-kind, and outcome fields; filter-chain events
also include elapsed microseconds. Record payloads and context values are never
included. They target `femtologging::internal`, which `FemtoTracingLayer`
ignores to prevent tracing from recursively logging its own instrumentation.

### Flush observability

Every `FemtoLogger::flush_handlers()` call records one bounded outcome and one
duration in process-wide, cumulative counters. Publishing them is deferred to
[issue 551](https://github.com/leynos/femtologging/issues/551), which owns the
supported telemetry interface, the exporter, and the alerting documentation;
the names below are the internal contract that exporter must match. Until it
lands, an operator reads flush health from the rate-limited warnings described
below, not from a metric endpoint.

| Metric                               | Type      | Meaning                                                                                                                                               |
| ------------------------------------ | --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `femtologging.flush.total{outcome}`  | counter   | Flush attempts by outcome: `success`, `ack_timeout`, `channel_closed`, or `handler_failure`.                                                          |
| `femtologging.flush.duration_us`     | histogram | Flush duration in microseconds, with cumulative buckets at 100, 500, 1,000, 5,000, 10,000, 50,000, 100,000, and 1,000,000 µs plus an overflow bucket. |
| `femtologging.flush.bridge_failures` | counter   | Logger flushes that returned `false` while `log::Log::flush` swept the registered loggers.                                                            |

The labels above are the contract the exporter must publish; until issue 551
lands, no metric endpoint exposes them. An operator reads flush health today
from two signals that do exist: the rate-limited warnings described below, and,
with `tracing-compat`, the `logger_flush` span, whose `flush_outcome` field
carries these labels. Each label means the following. `ack_timeout` means the
shard worker did not finish the records queued ahead within the shared
two-second barrier, which usually points at a handler that blocks.
`channel_closed` means the dispatch pool was already gone, so nothing was
drained. `handler_failure` means the drain succeeded but at least one attached
handler's `flush()` reported failure. `success` is the only outcome after which
every record queued before the call had been handed to its handlers. The
bridge-failure count isolates failures observed through `log::Log::flush`,
without naming the logger, because logger names are unbounded user data.

A Python handler that fails to flush is reported with one bounded category:
`flush_lookup_failed`, `flush_call_failed`, or `flush_result_false`. The
handler's exception message and traceback are never printed or logged, because
they may quote a record.

With `tracing-compat` enabled, each flush also emits a `logger_flush` span
targeting `femtologging::internal`. The span carries
`operation = "logger_flush"` and records `flush_outcome` and `elapsed_us`; the
outcome values match the counter labels. Acknowledgement timeouts, handler
failures, and bridge failures also emit rate-limited warnings (at most one per
five seconds per boundary) that report only a count. Neither the span nor the
warnings include record payloads or logger names, because both are unbounded
user data.

### Python-facing `flush()` and `close()` waits

Every built-in handler runs its I/O on a worker thread. The Python bindings of
the stream, file, rotating-file, timed-rotating-file, socket, and HTTP handlers
release the GIL before waiting for that worker in `flush()` and `close()` — the
socket and HTTP bindings through `detach_for_wait`, the other families through
a direct `Python::detach()` — so a Python callable formatter queued on the
worker can run to completion instead of deadlocking against a producer that
still holds the GIL, and other Python threads keep running during a slow
network shutdown. For the stream, file, rotating-file, and timed-rotating-file
handlers, `close()` takes only a shared Python-object borrow: it copies the
shutdown state and the owning worker handle out of the handler, releases the
borrow, then detaches and waits outside it. A Python callback that re-enters
the handler during shutdown therefore observes the closed state instead of
blocking on a borrow held by the waiter. `tests/test_handler_close_reentry.py`
is the regression test for that path.

`FemtoLogger.flush_handlers()` follows the same GIL rule, because the dispatch
worker takes the GIL to reach handlers attached from Python. It then flushes
every configured handler, and a handler registered from Python is flushed
through `PyHandler::flush()`. A built-in handler is flushed through its native
implementation with the GIL detached, and a user-defined handler is asked to
flush through its own `flush` method, which runs with the interpreter attached,
so whether other Python threads run while it blocks depends on the operation it
blocks on. Those calls run on the thread that called `flush_handlers()`, after
the dispatch worker has drained the logger's queue, not on the dispatch worker
itself.

Every handler is flushed even when an earlier one fails, and the results are
combined afterwards; one handler's failure must not leave the records buffered
in the handlers after it. `flush_configured_handlers` snapshots the attached
handlers under the handler read guard and drops the guard before it calls any
of them. That ordering is the invariant, not an implementation detail: a Python
`flush` may call back into the logger, and a guard held across the callback
would leave a writer waiting for a lock the callback cannot release. A missing
or non-callable `flush` method counts as success. A `flush` that returns `None`
also succeeds, because Python's flush protocol reports failure by raising; any
other falsy return value, `False` and `0` included, reports failure, and so
does a `flush` that raises. A failed flush reports only a bounded category; the
handler's exception text is never printed or logged, for the reason given under
[Flush observability](#flush-observability).

The one path outside that rule is not a flush failure at all: when an attached
handler cannot be unwrapped into its built-in type, the extraction error takes
the ordinary handler-error path and is printed with its traceback, because it
reports a registration mistake rather than a `flush` that ran and failed. An
interpreter that cannot be attached, which happens only while the process is
shutting down, leaves the handler unflushed and reports
`interpreter_unavailable`.

The warnings do not all carry the same fields, and the difference is
deliberate. A built-in handler that reports a failed native flush logs
`handler_kind=<type>, category=flush_failed`, where the type is `stream`,
`file`, `rotating-file`, `timed-rotating-file`, `socket` or `http`. An
interpreter that could not be attached logs
`handler_kind=<type>, category=interpreter_unavailable`, and there the field
may read `custom`, because that failure can reach a user-defined handler too. A
user-defined handler whose own `flush` ran and failed logs a fixed message that
carries only the category, `flush_lookup_failed`, `flush_call_failed` or
`flush_result_false`; it has no `handler_kind` field, because the category
already says the handler is custom and which step failed. Under the
`tracing-compat` feature every flush also runs inside an `info_span!` named
`flush_registered_handler`, which records `handler_kind` — the type label
above, or `custom` — and `flush_outcome`, but no failure category, so the
queue-crossing wait appears as a timed span rather than only as an event
emitted after it returns. `tests/test_custom_handler_flush.py` pins the
reported results and delivery, `tests/test_flush_releases_gil.py` pins the
formatter and flush cases, and `tests/test_network_handler_gil_release.py` pins
the network shutdown case.

Because the shutdown state is shared rather than moved out by an exclusive
borrow, several threads may close the same handler, and a close may start while
a detached flush is still running. Each closer waits on one completion
broadcast: every caller waits on the completion signal the worker raises as its
body ends, and a closer that arrives after that signal returns without waiting
out its timeout. A repeated `close()` is therefore safe: it returns once the
signal is observed, or when its own wait for that signal times out. A closer
that times out observes neither the signal nor the worker's state, and
`close()` returns no status by which the caller could tell the cases apart. The
signal is not the thread's exit: thread-local destructors run after the worker
body returns, so the thread can outlive a closer that is no longer waiting for
it.

Detaching preserves the existing return values and bounds. `flush()` still
returns `True` only when the worker acknowledges within the handler's flush
timeout and `False` otherwise, and `close()` still returns `None`. The file
family's `close()` wait is fixed at one second, not the overflow policy's
timeout, while the stream handler's uses its configurable `flush_timeout`
(default one second). A worker that does not signal completion within that
bound is reported and skipped rather than waited on indefinitely, and the call
returns while the worker may still be running.

### Handler wait diagnostics

Each Python-facing wait on a worker-backed file handler records one bounded
counter when it returns. `FemtoFileHandler`, `FemtoRotatingFileHandler`, and
`FemtoTimedRotatingFileHandler` expose `wait_diagnostics()`, which returns a
dictionary with a fixed key set: one counter per outcome (`handle_accepted`,
`handle_queue_full`, `handle_timed_out`, `handle_closed`, `handle_disconnected`,
`handle_failed`, `flush_acknowledged`, `flush_enqueue_timed_out`,
`flush_ack_timed_out`, `flush_closed`, `flush_disconnected`, `flush_failed`,
`close_completed`, `close_timed_out`, `close_panicked`, `close_unknown`) plus
`total_wait_micros`, `max_wait_micros`, and `waits_in_flight`. Counters use
relaxed atomics: the handler path must not pay for cross-thread ordering, and a
slightly stale snapshot is acceptable for periodic reporting.

`waits_in_flight` is a gauge rather than a counter. It rises before a wait
detaches and falls again when that wait returns, so a reader can tell an idle
handler from one whose producer is parked on a full queue or whose closer is
still waiting for the worker. Every detached wait in all three bindings is
bracketed by an RAII guard that decrements it, including an early return or a
panic unwinding through the binding; a wait that never returns is exactly the
state the gauge exists to expose. Because it is a gauge it does not accumulate:
successive snapshots are comparable directly, unlike the outcome counters.

With the `tracing-compat` feature enabled, each wait also emits a trace event
targeting `femtologging::internal` with `operation` (`handler_handle`,
`handler_flush`, or `handler_close`), `handler_kind` (`file`, `rotating`, or
`timed_rotating`), `policy_bucket` (`drop`, `block`, or `timeout`),
`wait_outcome`, and `elapsed_us`. The event is emitted after the detached
section returns, so it runs on the interpreter-owning thread; a tracing bridge
never has to attach to Python from a thread that holds no GIL. Neither the
counters nor the events carry logger names, paths, messages, payloads, or raw
timeout values, so their cardinality stays bounded however much traffic the
handler sees. `tests/test_wait_diagnostics.py` pins the key set and the outcome
semantics for all three handler kinds.

Activation is per accessor and per build. `wait_diagnostics()` always works and
needs no configuration; the trace events exist only when the crate is built with
`tracing-compat`, where each event costs one `tracing` call on the interpreter
thread. In a build without that feature `emit_event` is a compiled no-op, so
the only per-wait cost is the counter update: one relaxed `fetch_add` when the
wait is marked in flight, and on return one relaxed `fetch_sub` for the gauge
plus two relaxed `fetch_add` calls and one relaxed `fetch_max` for the outcome
and durations, with no allocation, formatting, or lock. The timer also reads
the clock twice, once before the detached section and once after it, so
`elapsed_us` includes the time taken to reattach to the interpreter as well as
the wait itself. No benchmark has measured this overhead; treat the cost
description as a design statement rather than a measured figure. Readings are
cumulative for the life of the handler, so an operator reports a delta between
two snapshots rather than reinterpreting a total.

The counters are diagnostic signals, not error rates:

- `handle_queue_full` and `handle_timed_out` rising against
  `handle_accepted` mean producers outrun the worker under a `drop` or
  `timeout` policy, and records are being lost.
- `handle_closed` is an expected post-shutdown call. `handle_disconnected`
  means the worker's channel disappeared while the handler still looked open,
  which points at a worker that died rather than at a deliberate close.
- `flush_failed` points at the file or filesystem. `flush_enqueue_timed_out`
  means the request never reached the worker because the queue was saturated;
  `flush_ack_timed_out` means the queue accepted the request but the worker did
  not answer in time. Acceptance by the queue is not acceptance by the worker:
  the worker may still be formatting or writing an earlier record and may not
  have read the flush command at all. Neither outcome implies the records were
  lost.
- `flush_closed` is an expected post-shutdown call; `flush_disconnected` again
  points at a worker that died.
- `close_timed_out` means the worker did not signal completion within the
  one-second bound, so queued records may still be unwritten. `close_panicked`
  means the worker signalled completion but its thread had panicked, so the
  final flush may not have run. `close_unknown` means completion was observed
  but no verdict was recorded. A worker that installs its exit guard records a
  verdict before signalling completion, so this is the initial value and a
  defensive fallback rather than an expected outcome.
- `waits_in_flight` is a snapshot, not a verdict. A value above zero means at
  least one caller was still inside a wait when the snapshot was taken: a
  producer not yet accepted, a flush still unacknowledged, or a closer still
  waiting. A zero reading does not identify the cause of latency that has
  already ended, and a non-zero reading does not prove the worker has stopped
  for good: a `handle()` blocked behind a busy worker keeps the gauge above
  zero after a `close()` times out, and that worker may still recover and
  drain. Sample the gauge over time and read it beside the outcome counters
  rather than treating one reading as a diagnosis.
- A `max_wait_micros` close to `total_wait_micros` with few operations
  suggests one slow wait rather than sustained pressure; the reverse suggests
  steady queueing.

`wait_outcomes.rs` owns the classification these counters rely on, so adding a
new outcome is a single match arm plus one key, and the fixed-key test fails
until the Python surface and its stub agree.

## Configuration transaction

`ConfigBuilder::build_and_init` splits into a fallible preparation step and a
commit that applies the result in one transition, so the operation is atomic
from the caller's perspective. Preparation validates the schema version and
root logger, validates every configured logger name before constructing any
handler, builds the handler and filter registries, and resolves every logger
plan, rejecting unknown or duplicate identifiers and Python-backed handlers
that cannot dispatch contextually. It then calls `stage_loggers`, which pairs
each plan with an existing logger or a new unregistered logger object while
only reading the registry. All of this work runs before any live state changes,
so a preparation failure leaves every pre-existing logger untouched.

The resolved state is assembled into a `PreparedConfiguration`, which
`manager::commit_configuration` applies under a single manager write lock. The
only fallible work under that lock is `register_missing_ancestors`: it
recomputes the planned names and their missing dotted ancestors, rechecks
`registry_has_room_for` against the live registry so room consumed since
preparation is counted, and builds every missing ancestor object before
inserting any. A refusal therefore raises the registry-limit `RuntimeError`
without mutating anything, and all of this happens before the first live-logger
change. The refusal path also releases the manager lock and then the GIL before
the staged loggers and registries are dropped, because their handlers may flush
or join a worker during teardown. The commit then adopts each staged logger,
replacing handlers and filters, applying level and propagate, and calling
`set_disabled(false)` only on the loggers the configuration names explicitly. A
logger registered by another caller between preparation and commit is adopted
instead, and the unused staged object is retained for teardown.

When `disable_existing_loggers` is on, the commit then disables every
registered logger outside `root` and the keep set, clearing each disabled
logger's handler attachments and filters. The keep set is the explicitly
configured logger names, plus `root`, plus every dotted ancestor of a
configured name. Kept ancestors retain their existing enabled or disabled state
and their attachments, so a configured descendant can still propagate to them,
but keep-set membership does not re-enable an ancestor that was already
disabled. Only a logger named explicitly in the configuration is re-enabled.
The commit finishes by publishing one `RuntimeStateSnapshot` holding the
handler registry, handler filter IDs, filter registry, and per-logger
attachment metadata. Handlers, filters, unused staged loggers, and the
displaced previous snapshot are retained in `DisplacedAttachments`; after the
manager lock is dropped, `build_and_init` calls
`displaced.drop_without_gil(py)`, so handler teardown that flushes or joins a
worker runs without the GIL.

`dictConfig` and `fileConfig` use this same builder path, so their validation
and transaction boundary match direct builder use.

At runtime, each handler is paired with its resolved filter chain in a
`HandlerAttachment`. The logger evaluates that chain on the producer thread
before enqueueing the record, including when a descendant record propagates to
an ancestor handler. Chains run in configured attachment order against an
evolving record. Each accepted handler gets a `HandlerRecordSnapshot` after its
own chain: later handlers see enrichment from earlier chains, while enrichment
from later chains cannot mutate snapshots already captured for earlier
handlers. Worker threads perform queued handler delivery; filter callbacks
remain on the producer thread for femtologging-configured handler filter
chains. This does not make Python-backed handler workers Python-free: under the
captured context, their synchronous callbacks can invoke Python, including a
wrapped stdlib handler's filters, formatter, and `emit()` invocation. Work
deferred beyond those callbacks requires separate context propagation.

Handler filter IDs are resolved during configuration using the same filter
registry as logger filters. `ConfigBuilder` resolves builder filter IDs before
attaching handlers, and `dictConfig` translates each handler's `filters` list
through that configuration path. Duplicate handler filter IDs collapse in
first-seen order, while unknown IDs fail validation before the live logger
state is changed. Standalone handler builds reject unresolved filter IDs
because they do not have a configuration registry.

Formatter resolution has two paths. Passing a `FormatterBuilder` directly to a
handler builder compiles that formatter for the handler. Passing a formatter ID
instead defers resolution to `ConfigBuilder`, which resolves the registered
builder during configuration; `dictConfig` formatter definitions use the same
registered-ID path. A configured percent formatter reads standard record fields
and structured values from `metadata.key_values`. Formatter objects and filters
are therefore both resolved before the configuration transaction commits.

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
- `json_round_trip.rs` provides `assert_json_round_trip`, a `#[track_caller]`
  assertion that a serde value survives a JSON round trip unchanged and names
  the direction that failed.
- `logging_test_helpers.rs` provides the `unique_logger_name` and
  `log_max_level` fixtures and `run_isolated_child`, which reruns one ignored
  test in a child process and returns `io::Result<()>` rather than panicking.
  Its items carry narrower feature gates than the module, so a suite should
  take only the ones its own feature set provides.
- `isolated_child.rs` launches that child, preserving the dynamic-library
  search path Cargo supplied to the parent.

One further data file is test-only without being a module:
`rust_extension/src/logger/producer_tracing_expected_tables.rs` holds the
expected tracing event literals, which `producer_tracing_test_helpers` splices
in with `include!` so the array keeps its compile-time type. It therefore has no
`//!` header; the comment at its top explains the split instead.

The reusable integration-test support is owned by
`rust_extension/tests/test_utils/`. It consists of four focused components:

- `handle_expect.rs` defines the `HandleExpect` trait, which turns a handler's
  fallible `handle` call into a descriptive test panic.
- `fixtures.rs` provides `handler_tuple` for a fresh buffer and default
  stream handler, and `stream_handler_for` when several handlers must share one
  buffer.
- `shared_buffer.rs` provides the standard-library shared buffer, while
  `shared_buffer_loom.rs` provides its Loom-backed counterpart.
- `captured_log.rs` provides `captured_log()`, the process-wide capture logger
  described below.

Each Cargo integration-test root declares only the support modules it needs.
The stream-handler suite includes `test_utils/mod.rs` because it uses all of
them; the file-handler and logger suites include their required files directly.
The `heavy` root includes `shared_buffer.rs`, `shared_buffer_loom.rs`, and
`handle_expect.rs` directly; the Loom module is gated by `cfg(loom)`. Prefer
these fixtures and the trait over duplicating setup or
`handle(...).expect(...)` calls in individual suites.

Setup helpers return the error their step produced — `io::Error`, `ConfigError`,
`HandlerError` or `PyResult` — and the calling test propagates it with a
reasoned `expect`. A failing arrangement then names the step that failed
instead of surfacing as a panic from inside the helper.

### Capturing log records in an integration test

`log::set_logger` succeeds once per process and returns an error for every
later call, which `logtest::start()` unwraps. Two tests in one binary that each
called it therefore could not both run: whichever ran second panicked, and only
which one lost varied with the order the harness chose. `captured_log()`
installs the logger once behind a `OnceLock` and hands out a
`MutexGuard<'static, logtest::Logger>`, so a second caller waits rather than
panicking.

Installing once is not enough on its own. `logtest::Logger` holds no state of
its own; every captured record lives in a queue inside `logtest` that outlives
the test that produced it, and a test that panics part way through leaves its
records behind. `captured_log()` therefore drains the queue under the same lock
that hands out the logger, so the drain and the use are one operation rather
than two an ordering could separate. A lock poisoned by a panic elsewhere is
recovered rather than propagated, because that test has already failed and a
second panic here would hide the first.

Two rules follow for callers, and both matter:

- **Hold the guard for as long as the logger is needed.** Dropping it early
  lets another test drain the records out from under the first.
- **Match records by their text, not only by their level.** The capture is
  process-wide while the mutex scopes only the callers that take it, so "the
  queue holds one warning" is a claim about every record the binary emits. Name
  the message under test instead.

Tests that use it carry `#[serial]` and `#[ignore]`, and the scheduled
`heavy-tests` lane runs them with `-- --ignored` so nothing else in the binary
emits alongside them. Under `--include-ignored` the ordinary tests in the same
file run too and do emit, which is why the assertions name their records.

## Structured logging contracts

Python logging captures the active task-local `ContextVar` fields and any
explicit `extra` fields when `FemtoLogger.py_log` creates a record. The
`rust_extension/src/python_context.rs` adapter converts Python mappings and
scalar values, then `ContextBudget` in
`rust_extension/src/log_context/budget.rs` validates keys, values, and
aggregate size; `rust_extension/src/log_context.rs` re-exports it alongside the
scoped-context API. Budget validation checks key length, then the reserved-name
contract, then key count, value length, and aggregate size, and commits state
only after every check passes. The resulting fields are stored in record
metadata; this path does not read or merge Rust thread-local context.
Conversion failures are reported to Python as type errors, and invalid context
values as `ValueError`.

Reserved names are defined once, in
`rust_extension/src/log_context/reserved.rs`. Context admission, Python
callback enrichment, and HTTP serialization all call the same predicate, so a
name that cannot enter through `extra` or `log_context(...)` also cannot
replace an authoritative field later in the pipeline. Because
`RecordMetadata::key_values` is public and direct construction bypasses
admission, HTTP serialization omits reserved names at the output boundary as
well; that omission is defence in depth, not a second policy.

Rust logging uses `merge_context_values` at the producer boundary. It combines
active Rust thread-local context with explicit record fields, with explicit
fields taking precedence on collisions. Rust macros and records received
through the `log` and `tracing` bridges use this merge; invalid context causes
the record to be dropped rather than silently accepted.

`PyHandler` inspects handler capabilities when a Python handler is registered.
For built-in stream, file, rotating-file, timed-rotating-file, socket, and HTTP
handlers, native dispatch passes the original `FemtoLogRecord` to the
`FemtoHandlerTrait` implementation, preserving `RecordMetadata.key_values`
through the handler queue. Other Python handlers use the structured
`handle_record` dictionary interface when available, or the legacy three-
argument `handle` interface as a fallback. This boundary is described in more
detail in [the Rust extension design notes](rust-extension.md).

File-handler unit tests use `rust_extension/src/handlers/file/test_support.rs`.
The `impl_unsupported_seek!` macro supplies the required `Seek` implementation
for an unseekable test writer and consistently returns
`io::ErrorKind::Unsupported`; the same module also provides the process-wide
test logger and helpers for installing it and taking captured messages.

The macro unit tests in `rust_extension/src/logging_macros.rs` use the
`logger_with_handler` `rstest` fixture. It clears the test logging context,
creates a DEBUG-level `FemtoLogger`, and attaches a `CollectingHandler` so each
macro case can inspect the resulting record.

`rust_extension/src/test_fixtures/explicit_traceback.py` is Python source data,
not a package module. `traceback_capture_tests` embeds it with `include_str!`;
the fixture raises a nested `ValueError`, preserves its explicit traceback, and
clears the exception object's `__traceback__` so tuple-based traceback capture
is exercised.

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

`femtologging` runs Python linting as five tiers, all reachable through
`make lint` (`lint-python`, followed by `lint-rust`). Each stage must pass
before the next runs. The original four-tier decision is recorded in
[ADR-0005: Four-tier Python lint architecture](adrs/adr-0005-four-tier-python-lint-architecture.md);
its addendum records the addition of the docstring-coverage tier below.

1. **Ruff** — fast, broad rule set in preview mode, targeting `py312`. Pinned
   by `RUFF_VERSION` (`0.16.4`); see [Ruff version](#ruff-version) for the
   pin-sync scheme with CI.
2. **interrogate** (`1.7.0`) — enforces 100% docstring coverage over
   `INTERROGATE_TARGETS`: the package, the tests (pytest-bdd step functions and
   nested helper closures included), the scripts, the examples, the Rust
   extension's Python fixture and `conftest.py`; `.pyi` stubs are not scanned.
   The `interrogate` target prints every scanned definition and fails on any
   `MISSED` row, because interrogate rounds its percentage to one decimal and
   the threshold alone can pass a lone gap. `INTERROGATE_IGNORE_REGEX`
   (`^basicConfig$`) skips every definition named `basicConfig`, which is how
   the overload stubs are exempt; Ruff still checks the implementation's
   docstring. Ruff's `undocumented-public-function` (D103) also applies to
   `tests/steps/` without a per-file exemption. A step's docstring describes
   its scenario-context effect or assertion contract rather than repeating the
   `@given`/`@when`/`@then` decorator text.
3. **Pylint** (`4.0.7`) — runs as a uv tool on managed PyPy 3.12
   (`PYLINT_PYTHON ?= pypy@3.12`), isolated from the project virtual
   environment. `syntax-error` stays enabled, so a module PyPy cannot parse
   fails the lint rather than being skipped. Configuration lives in
   `pyproject.toml`'s `[tool.pylint]` tables: `py-version = "3.12"`,
   `max-module-lines = 400`, and a curated `enable` list covering logging
   interpolation, pattern matching, generator control flow, environment
   handling, and subprocess safety.
4. **`df12-python-lints`** and its companion **`ambrleaks`** — run under
   CPython 3.14 so the house-rule parser stays ahead of the project's 3.12
   syntax baseline. `df12-python-lints` is pinned to a specific commit of the
   `v0.3.0` release and enables the message set
   `R9101,C9102,R9103,R9104,C9105,C9106,C9107,R9108,R9109,R9110,R9111,R9112,C9112`.
   `ambrleaks` sweeps Syrupy `.ambr` snapshots under `tests` and
   `femtologging/unittests` for unredacted secrets.
5. **Skylos** (through `df12-skylos`) — a blocking production dead-code gate,
   run under Python 3.14 so Skylos parses the project's syntax with its own
   runtime `ast` implementation rather than an older one that could produce
   phantom findings. See [Skylos dead-code gate](#skylos-dead-code-gate) below.

Run the full lint gate with:

```shell
make lint
```

To run any of the five tiers locally, invoke the underlying tool directly:

```shell


# Tier 1: Ruff
uvx ruff==0.16.4 check


# Tier 2: interrogate (docstring coverage, every Python tree the repository owns)
make interrogate


# Tier 3: Pylint under managed PyPy
uv tool run --managed-python --python pypy@3.12 --from 'pylint==4.0.7' pylint \
  femtologging tests scripts


# Tier 4: df12-python-lints
uv tool run --python 3.14 --from 'pylint==4.0.7' \
  --with 'git+https://github.com/leynos/df12-python-lints.git@4cf41736cce2f7ba2778882a5c629c044568a0e5' \
  pylint --disable=all --load-plugins=df12_python_lints \
  --enable=R9101,C9102,R9103,R9104,C9105,C9106,C9107,R9108,R9109,R9110,R9111,R9112,C9112 \
  femtologging tests scripts


# Tier 4: ambrleaks (same df12-python-lints package, different entry point)
uv tool run --python 3.14 \
  --from 'git+https://github.com/leynos/df12-python-lints.git@4cf41736cce2f7ba2778882a5c629c044568a0e5' \
  ambrleaks tests femtologging/unittests


# Tier 5: Skylos
make skylos   # df12-skylos check --repository .
```

Prefer running the exact Makefile-derived commands (for example,
`make lint-python`) over hand-copied invocations, since the Makefile is the
single source of truth for tool pins, targets, and flags.

### Skylos dead-code gate

Skylos analyses production code only: `femtologging/unittests` and the native
`femtologging/_femtologging_rs.pyi` stub are excluded, so test-only references
cannot keep a production symbol alive and the stub's inherently "unused" native
parameters never trigger findings. `--no-grep-verify` prevents a
repository-wide text match from masking a genuinely dead production symbol, and
`[tool.skylos.gate] strict = true` in `pyproject.toml` enforces the strict gate.

Investigate every Skylos finding before suppressing it:

- **Genuine dead code** must be removed.
- A **verified false positive** — an implicit runtime caller such as a
  re-exported native module, a test-util hook, or a protocol-shaped parameter —
  should first be modelled as a typed `[[tool.skylos.dead_code.entrypoints]]`
  rule in `pyproject.toml`, giving the fully qualified symbol, its `type` (for
  example, `"import"`, `"variable"`, or `"parameter"`; use `"method"` for
  methods), and a caller-specific reason.
- Only when an entry-point rule cannot describe the boundary should a named
  allow-list exception be recorded:

  ```bash
  make skylos-allow SYMBOL=handler REASON="Loaded by plugin registry"
  ```

  Both `SYMBOL` and `REASON` are required; the target rejects empty or
  whitespace-only values with exit code 2. Use `SYMBOL` rather than `NAME`,
  because Windows Subsystem for Linux (WSL) may inject `NAME` with the host
  name. `df12-skylos allow` serializes whitelist writes with a lock on the
  ignored `.pyproject.toml.df12.lock` file, so concurrent invocations do not
  overwrite one another. Never record a broad or unreasoned exception.

Skylos runs through the installed `df12-skylos` command from df12-python-lints,
provisioned at the commit in `DF12_SKYLOS_SHA` (its `skylos` extra carries the
one Skylos pin). The command's behaviour is tested in df12-python-lints, and CI
runs the shared-actions `df12-skylos` action. This repository keeps the
production roots (`[tool.df12_skylos]`), the exclusions
(`[tool.skylos] exclude`), the strict gate and its entry-point rules in
`pyproject.toml`.

### Makeutil bootstrap

`makeutil` is a prerequisite of `make test` (the `test` target depends on the
`makeutil` target, which only verifies the executable is present) and of every
full-suite CI job — `ci.yml`'s `build-test` job and `heavy-tests.yml`'s `heavy`
job each install it with the shared `install-makeutil` action, which downloads
a prebuilt release and checks it against a pinned digest and the release's own
`.sha256` file, then smoke-test it in a `Verify makeutil` step. The assertions
for both live in `tests/makeutil_contract.py`, called from
`tests/test_makeutil_ci_contract.py`; bump the pinned action reference there
and in the workflows together. Before running `make test` locally, download
`makeutil-x86_64-unknown-linux-musl` (or the `aarch64` build) from the release
the `install-makeutil` action defaults to, listed at
<https://github.com/leynos/makeutil/releases>, verify it against the matching
`.sha256` file, and put it on `PATH` as `makeutil`.

## Environment Access in the Rust Extension

The Rust extension must not read or mutate the process environment ambiently.
`rust_extension/clippy.toml` disallows `std::env::var`, `var_os`, `vars`,
`vars_os`, `set_var`, and `remove_var`, and `rust_extension/Cargo.toml` denies
`clippy::disallowed_methods`. The `lint-env-policy` target, which `lint-rust`
runs first, applies that lint with `--all-targets` over one lane per feature:
`none` for `--no-default-features`, `all` for `--all-features`, and one lane
per declared feature enabling that feature alone. `--all-features` on its own
would never compile a `#[cfg(not(feature = ...))]` block, so the `none` lane is
not optional.

The lanes are walked by `scripts/lint_rust_lanes.py`, which follows the estate
scripting standards: a `uv` script block, Cyclopts reading `INPUT_`-prefixed
environment variables, and plumbum invoking Cargo. It exits non-zero on the
first failing lane and names it. That job used to be a shell `for` loop in the
Makefile, which reports only its last command's status and so let an earlier
lane's rejection pass silently. `make lint-lanes-test` runs the driver's own
tests, which use `cmd-mox` to shim `cargo`, and `lint-rust` depends on it so
the driver is proven before it is trusted.

Adding a feature to `rust_extension/Cargo.toml` means adding its lane;
`rust_extension/tests/env_access_policy.rs` derives the required lanes from the
manifest and fails otherwise.

Pick the lightest seam the boundary justifies:

- an explicit value, for one-off configuration;
- a narrow reader closure, for a small reusable boundary;
- a shared environment trait, only when several variables and tests justify it.

Tests never change the parent process environment. A test that needs the code
under test to observe a value passes it through the seam. A test that needs a
child process to see a variable builds the child's environment explicitly.
`Command::env` alone keeps the parent's environment, so where an inherited
variable could change the outcome, call `Command::env_clear` first and then add
back every variable the child legitimately needs, `PATH` included.

Serialization, whether `#[serial]` or a lowered `TEST_THREADS`, stays only
where a structural reason such as the global manager or the Python interpreter
demands it.

A direct read is permitted only at a genuine composition root, under an
item-scoped `#[expect(clippy::disallowed_methods, reason = "...")]`. Use
`expect`, not `allow`, so the attribute warns once the site is migrated.

`rust_extension/tests/env_access_policy.rs` holds this configuration to that
contract. See
[adrs/adr-0006-environment-seam-taxonomy.md](./adrs/adr-0006-environment-seam-taxonomy.md)
for the decision and its rationale.

### The policy contract test

`tests/test_env_access_policy_contract.py` judges the target's wiring from
outside: it runs the real `make lint-env-policy` target, so Make, `uv` and the
lane driver all execute. It then checks that a failing lane fails the target,
that the driver names that lane, that every lane's diagnostic reaches the
caller on standard error, and that Cargo receives the exact argument vector the
Makefile's defaults imply, lane by lane and in order. Cargo itself is the one
part replaced: `tests/fake_cargo.py` writes a recording fake `cargo`, which
`run_policy_target` in `tests/env_policy_lane_support.py` puts first on the
child's `PATH`; its `cargo_runner.py` companion does the recording and rejects
only the lane a test names.

The fake compiles nothing, so the contract never takes Cargo's package-cache
lock and cannot wait on another job's resolve
([#537](https://github.com/leynos/femtologging/issues/537)). Each run
therefore has 120 seconds (`MAKE_TIMEOUT_SECONDS`) for provisioning through
`uv` and the real Make run, and each test carries the
`CONTRACT_TIMEOUT_SECONDS` marker above that budget. Real compilation and
policy enforcement stay with `rust_extension/tests/` and `make lint`.

The child inherits this process's environment so that `uv` and Make work, but
the helper removes every Make variable the Makefile assigns with `?=` before
running the target: such a variable takes the environment's value over the
Makefile's default, so an exported `ENV_POLICY_FEATURE_LANES` would change
which lanes run and an exported `ENV_POLICY_LINT_ARGS` would change what Clippy
is asked to do. `CARGO` is in that list because it is assigned with `?=` as
well, but the target never expands it into a recipe, so the fake is selected by
`PATH` either way. The list is read from the Makefile rather than written out,
so a new `?=` variable is covered when it is added. It also removes Make's own
controls, `MAKEFLAGS`, `GNUMAKEFLAGS` and `MAKEFILES`: `MAKEFLAGS=-i` would
make the target ignore a rejected lane's exit status, and either variable can
carry assignments the Makefile parse cannot see. The helper removes
`FAKE_CARGO_REJECT` before setting its own value too, since an inherited
rejection would fail a lane the test expects to accept, and the failure would
read as a wiring defect. The general rule is the one above — where an inherited
variable could change the outcome, the child's environment is built explicitly.
`run_policy_target` takes the parent environment as an argument for the same
reason, so those contracts inject one instead of editing this process's
environment, and it runs the target in its own process group so a timed-out or
interrupted run cannot leave `uv` and the driver behind.

These invariants are checked with named examples and one parameterized table
rather than generated ones, deliberately. Each example runs the real target, so
a property test would multiply whole `uv` and driver runs to sample an input
space that is not open-ended: the inherited names are exactly the variables the
Makefile assigns with `?=` plus Make's three control variables, and
`environment_overridable_variables()` already enumerates that list in full. A
generated value adds nothing a case over that list does not already assert, and
a generated subset of lanes would re-test the driver's loop rather than the
contract, which pins the loop at its stop-at-first-failure boundary. The
relative and empty `PATH` entries are covered by a parameterized table for the
same reason: the interesting cases are two, and both are named.

See
[adrs/adr-0006-environment-seam-taxonomy.md](./adrs/adr-0006-environment-seam-taxonomy.md)
for the decision, and the
[2026-09-08 debugging plan revision](./debugging/debugging-plan-2026-09-08-env-policy-timeout.md)
for the measurements that led to the fake and the alternatives that were
rejected.

### The source scan

Clippy cannot see an attribute that switches its own lint off, so
`rust_extension/tests/env_policy_source_scan.rs` reads the crate's sources and
rejects any suppression of the policy; its machinery lives in
`rust_extension/tests/test_utils/source_scan.rs` and the modules beside it. The
walk starts at the crate directory and reads every `.rs` file it can reach, so
a build script, a bench, an example or a second binary is governed wherever it
appears. `lint-env-policy` passes `--all-targets`, which compiles an `examples`
target when one exists, and no list of directories written today governs one
nobody has added yet.

It skips `target` and every dot-prefixed directory, being build output and tool
state rather than anywhere code Cargo compiles is written. `src`, `tests` and
`benches` remain named, as a tripwire rather than a filter: each must be
represented among the sources the walk returned, so a walk that silently read
nothing reports a failure instead of a clean crate. A directory that cannot be
read is a failure, not an empty result.

Run it with
`cargo test --manifest-path rust_extension/Cargo.toml
--no-default-features --test env_policy_source_scan`,
or as part of `make test`. It needs four development dependencies, all
declared in `rust_extension/Cargo.toml`:

*Table: Development dependencies the source scan needs.*

| Crate         | Why it is needed                                             |
| ------------- | ------------------------------------------------------------ |
| `syn`         | Parses each source and walks its attributes with a visitor   |
| `proc-macro2` | Walks `macro_rules!` token streams, which `syn` keeps opaque |
| `cap-std`     | Reads sources through a capability-scoped directory handle   |
| `camino`      | Carries the UTF-8 paths those handles return                 |

Sources are read through a `cap_std` `fs_utf8::Dir` handle rather than
`std::fs`, which the Dylint suite disallows, and each directory is opened
through its parent's handle so the walk cannot leave the tree it was given.
`include_str!` is not an option here as it is for the configuration contracts,
because the scan has to see files that do not exist yet.

Three shapes are refused structurally rather than by their meta, because none
of them is a suppression the scan can read where it is written. A
`macro_rules!` arm that forwards an attribute's *path* (`#[$attr]`) lets its
caller supply `allow`, so it is refused at inner scope, and at outer scope
where the arm writes an `env` access itself or forwards a fragment the caller
fills with code. The `$(#[$meta:meta])*` idiom for carrying doc comments onto a
generated setter is therefore untouched, as are `#[doc = $text]` and
`#[derive($traits)]`, whose paths are written out. And an `include!` is refused
unless its target is a literal `.rs` path *that the walk would reach*, since
`rustc` parses an included file as Rust whatever its extension and resolves it
against the file that writes it; `include_str!` and `include_bytes!` embed
bytes and are not inclusions. And `#[path = "../../bypass.rs"] mod bypass;`
names a module's source directly, compiling a file outside the crate directory
the walk reads, so it is judged by the same rule as an `include!` target: one
function, because an inclusion rule that drifted from a `#[path]` rule would
close one route and leave its twin open, which is how `#[path]` came to be
missing. A `path` reached through `cfg_attr` is judged too. The idiom this
suite itself uses, `#[path = "source_scan/discovery.rs"]`, names a `.rs` file
under a walkable directory and reports nothing.

That target is read as one parsed string literal and judged by its value, so a
raw string and an escaped dot name the same file a plain string does, and by
its extension as the walk selects sources, against the one shared constant, so
a bare `.rs` is a finding rather than an accepted target.

A `.rs` extension alone is not enough. `include!(".generated/bypass.rs")` names
a real Rust file under a directory the walk skips, so the target is resolved
against the including source and every directory it passes through is judged by
the same function the walk descends with. A target that climbs out of the crate
is refused for the same reason. Only the directories are judged, because the
walk selects a file by its extension alone and reads `.hidden.rs` like any
other.

The walk covers a `macro_rules!` transcriber, not the arguments of an ordinary
invocation, which the macro it is handed to may discard. That holds inside a
transcriber too: `stringify!(#[allow(clippy::all)])` writes an argument that
becomes a string, not an attribute. The forwarded-path rule is what makes the
narrowing safe: any attribute a macro emits has to be written in a transcriber
first.

Two readings keep the forwarded-path rule honest. An arm counts as writing an
environment access when it uses `env` as a path or a macro, `env::var` or
`env!`, not merely when some identifier is spelled `env`; a parameter of that
name is not an access. And a fragment specifier is looked for at any depth, so
`$($body:item)*` declares an `item` the way `$body:item` does.

An argument list the scan cannot read reports every protected lint rather than
none. `#[allow($lint)]` in a transcriber is a list `syn` accepts and whose
contents it cannot parse, and invoked as
`suppress!(clippy::disallowed_methods)` it expands to a real suppression. A
`cfg_attr` whose arguments will not parse is treated the same way, and reaches
the scan by the same route: an arm writing
`#[cfg_attr($cond, allow(clippy::disallowed_methods))]` forwards the condition
rather than the attribute's path, so the forwarded-path rule does not see it,
and the expansion suppresses the lint for real. Failing closed costs a
contributor who writes an unreadable argument an explanation; failing open
costs the policy.

The scan parses rather than searches. A text scan cannot follow `cfg_attr`,
cannot tell an attribute from attribute-shaped text in a string or a doc
comment, and breaks on a parenthesis inside a `reason`. It protects
`clippy::disallowed_methods`, its group `clippy::style`, the wider
`clippy::all`, and `warnings`, comparing lint names as whole paths with raw
identifiers normalized. An item-scoped `#[expect(..., reason = "...")]` is the
sanctioned form and passes, but only with a reason that is not blank, since the
reason is the whole of what distinguishes it from a quieter `allow`; a
crate-scoped `#![expect(...)]` does not, because one call anywhere in the crate
fulfils it and the rest go unreported.

## Structured logging context

Python `log_context(**fields)` stores a copied mapping in a `ContextVar`.
Python's context propagation gives each newly created asyncio task a snapshot
of the context active at task creation. Subsequent changes in the parent or
child remain local to that task. Python logger methods and module-level
functions copy the current fields into record metadata when they create the
record, before queueing it; later context changes do not affect that record.
Nested `log_context` scopes override same-named fields from outer scopes.

Rust scoped context uses an OS-thread-local stack. `LogContextGuard` is `!Send`
and `!Sync`, records its owner thread and a unique frame identity, and removes
only its own frame when dropped on that thread. Keep the guard within a
thread-affine synchronous scope; an async Rust task that can migrate between
threads must not retain this guard across an `await`.

The Python and Rust context stores are separate. Python entrypoints capture
Python task-local fields only. Rust macros and records received through the
`log` and `tracing` bridges merge Rust scoped fields at record creation.
Explicit key-values supplied by Rust macros and explicit event or span fields
captured by the tracing bridge override same-named scoped fields. Neither side
implicitly imports the other's context, so code crossing the boundary must pass
required metadata explicitly.

## Benchmarking Documentation

Benchmarking work is governed by
[benchmarking-and-optimization-design.md](./benchmarking-and-optimization-design.md)
and tracked in [roadmap.md](./roadmap.md). Keep those links intact when
editing developer workflow notes so contributors can move from toolchain setup
to the benchmarking phase design without losing context.

## Runner placement

`ci.yml`'s `build-test` runs on `ubicloud-standard-4`. `runs-on` selects it
with the runner-selection expression:

```yaml
runs-on: >-
  ${{ (github.event.pull_request.head.repo.fork
  || github.event.pull_request.user.login == 'dependabot[bot]')
  && 'ubuntu-latest' || 'ubicloud-standard-4' }}
```

It is `standard-4` rather than the estate's `standard-2` on a measured
shortfall: the first run on two vCPUs spent 23 minutes in lint, 9 in typecheck
and over 17 in the Rust tests, and was cancelled at its 55-minute ceiling (run
36596547457), against a hosted median of 23.5 minutes.

A pull request from a fork cannot obtain an Ubicloud runner, so it falls back to
`ubuntu-latest`. So does a pull request Dependabot opened: hosted minutes cost
nothing on a public repository, and Dependabot opens many pull requests. The
expression tests the pull request's author, not `github.actor`, because the
actor is whoever triggered the run, so a person pushing to a Dependabot branch
would flip the runner. A push and a dispatch have no pull request, so both
values are empty strings (falsy) and the expression selects Ubicloud; a
Dependabot pull request therefore builds cold on the hosted pool. `ci.yml` runs
on pull requests only, and Ubicloud's cache proxy is scoped by ref, so each
pull request warms its own scope. The one main-branch writer whose cache the
pull-request lanes read is `coverage-main.yml`'s baseline publisher, described
under [Coverage publication](#coverage-publication).

An Ubicloud runner is a self-hosted just-in-time runner, so GitHub's six-hour
cap for hosted jobs does not bound it and a hung job would hold a billable
runner. Every job whose `runs-on` can select Ubicloud therefore states its own
`timeout-minutes`, twice a measured warm Ubicloud run. `build-test` is at 40
minutes: warm `standard-4` runs took up to 19 minutes (run 36617135055,
attempts 2 and 3), against 21.4 on the first, cold-cache run.

The uv cache is the caller's. `generate-coverage` is given
`cache-provider: external`, because its own uv cache is keyed on the operating
system and the `pyproject.toml` hash alone and restores `environments-v2`,
whose entries are bound to the interpreter that built them: a warm restore of
that state made the action's `uv venv` exit 2 on Ubicloud
(leynos/shared-actions#547). The `Cache uv` step in `ci.yml` keys on the runner
environment and the lane's Python version and leaves `environments-v2` out of
the cached path, and `tests/test_uv_cache_contract.py` holds all three to the
file.

`tests/test_runner_placement_contract.py` holds this to the files. It evaluates
the expression for a push or dispatch, a same-repository pull request, a fork
and a Dependabot pull request, rejects a literal label, inverted arms, another
label, another condition, a missing Dependabot arm and an actor test, refuses a
workflow it cannot read by name, inventories and refuses a runner named through
the matrix (`runs-on: ${{ matrix.runner }}` over an Ubicloud value), and
asserts an exact inventory of the jobs that can land on Ubicloud with their
runner class and ceiling. A change that adds, removes or re-times such a job
fails it until the inventory is updated in the same commit.

## Coverage publication

CV-005 splits coverage in two. A pull request generates a Cobertura report and
measures against the stored baseline, but it publishes nothing and never
contacts CodeScene: `generate-coverage` runs with `with-ratchet: 'true'` and
`publish-artefact: 'false'`, and its job declares no environment. This change
also newly enables the ratchet on that lane, which previously ran the action
without `with-ratchet` and so took its `'false'` default, and it stops the lane
publishing its report. The default branch owns both persistent outputs, in
`coverage-main.yml`, which a push to `main` alone triggers. The decision, its
drivers and the alternatives it rejected are recorded in
[ADR-0009](adrs/adr-0009-main-branch-coverage-publication.md).

That publisher has four jobs. `coverage-measure` runs on a hosted runner once
per required Python lane (`3.12`, `3.13` and `3.14`, the non-experimental lanes
of `ci.yml`), installs `makeutil` because the measured suite parses the
Makefile through it, and hands each lane's figures to its publisher through a
`coverage-baseline-handover-<python>` artefact. `publish-baseline: never` keeps
the measuring runner out of the cache store entirely, because that store is not
the one the pull-request lanes read: a ratchet there could never fail and a
baseline saved there would never be restored. The baseline cache key carries
the interpreter's major.minor, so every required lane needs its own published
entry: a lane whose entry was never written restores nothing, compares against
zero and can never fail.

A cache miss still fails open. `generate-coverage` writes `0` into a baseline
file its restore did not supply
([leynos/shared-actions#699](https://github.com/leynos/shared-actions/issues/699)),
so an evicted entry starts that lane at zero and its ratchet cannot fail while
the run still looks green. The pinned action makes the miss visible but neither
this repository nor the action fails closed on it; the repair belongs in the
action. Its `Report ratchet baseline cache decisions` step prints
`metric ratchet-cache.restore=<hit|miss|skipped|error>`, raises a
`::notice title=generate-coverage ratchet cache::restore=… save=…` annotation,
and adds a `### generate-coverage ratchet cache` block with a
`- baseline restore:` line to the job summary, so the run log and the summary
record which comparison the lane actually made. `ci.yml` cites that line at the
ratchet input, because the baseline file cannot answer the question afterwards:
`ratchet_coverage.py` rewrites the file to the measured figure on the same run,
so a zero is no longer observable once the step has finished.

Two publishers then ratchet each lane's handed-over figures, because a cache
entry written on one runner provider is not read by the other.
`coverage-baseline` runs on an Ubicloud runner, where the same-repository
pull-request lanes run. `coverage-baseline-hosted` runs on `ubuntu-latest` and
writes GitHub's cache store, which is what a fork or Dependabot pull request
restores from: `ci.yml` places those lanes on the hosted pool, so without the
second publisher that lane would restore nothing, start its baseline at zero
and never fail. Each publisher ratchets the same figure and writes only its own
provider's store.

A failed lane must not withdraw a successful sibling's publication. Measurement
uses `fail-fast: false`, so every other lane still finishes, and each publisher
is conditioned on `!cancelled()` rather than on its dependency succeeding,
which is the one form that still runs after a failed `needs`. Each lane then
publishes only when the matrix as a whole succeeded or its own hand-over is
present: under a partial failure a `continue-on-error` probe decides, and an
absent hand-over means that lane measured nothing and publishes nothing. When
every lane succeeded the probe does not run, so a lost hand-over reaches the
action's own download unguarded and fails the publisher: that is a lost
publication, not an empty lane.

`coverage-upload` is a job of its own, downstream of the measurement alone, so
a CodeScene outage cannot stop the baseline advancing; it still fails, and
fails the workflow with it, when CodeScene refuses the report. It receives the
newest required lane's report as an artefact, because only that lane produces
one, and checks out the measured revision because the uploader takes its branch
and commit from the clone it runs in rather than from the report alone. A
failure in another lane therefore still uploads the trunk's report.

The upload is optional and guarded twice. A token-presence step writes
`available` and binds no secret; the upload step runs only when that output is
`'true'` and `github.ref` is `refs/heads/main`. The token reaches the uploader
as its `access-token` input and sits in no `env`, because the upload action is
composite and hands its step's environment to the nested steps it runs. The
presence step records the boolean and its consequence in the run log and the
job summary in both branches, so a skipped upload states its own reason, and it
never writes the secret. Until the owner provisions `CS_ACCESS_TOKEN` in the
`codescene` environment, the step reports `available=false` and skips; the
baseline job still runs, so a missing token cannot hide a coverage regression.

The `codescene` environment admits `main` alone, and no pull-request-reachable
job may declare it. The publisher's concurrency group is keyed on the ref and
never cancels, so a newer push cannot abandon an in-flight upload or baseline
write. `tests/test_coverage_publisher_contract.py` holds the whole split: both
publishers, one environment holder, the single recorded call into another
repository, the canonical guard and seven ways to weaken it, no token in any
`env`, and identical measurement inputs on both lanes.
`tests/test_coverage_reach_contract.py` holds the fail-closed trigger and
reachability readers, including the forwards direction of a `workflow_run`
edge: the listener becomes reachable once the workflow it watches does, not the
other way round.

The readers behind those contracts live in `tests/coverage_contract_helpers.py`,
`tests/coverage_reach_helpers.py`, `tests/coverage_environment_helpers.py` and
`tests/coverage_push_filter_helpers.py`. They are test-only readers: the
package imports none of them, and each fails closed on a declaration it cannot
resolve. That includes an expression-valued `env` and an aggregate `secrets`
access such as `toJSON(secrets)`, which are refused or reported as carrying the
token rather than passed as safe.

Both lanes must measure the same population, so every input that defines it
(`output-path`, `format`, `cargo-manifest`, `with-default-features`, `features`,
`pytest-workers`) is identical. The pull-request lanes run on Ubicloud and
restore from that provider's cache store, so `coverage-baseline` runs there
too; a push has no pull request, so both arms of the placement expression are
empty and it selects Ubicloud. The expression is kept because the placement
contract reads exactly that shape as the estate's fallback rule, and
`coverage-baseline-hosted` states `ubuntu-latest` literally.

Known exception: a Dependabot pull request merged by `dependabot-automerge` with
`GITHUB_TOKEN` fires no push event, so that merge publishes nothing until the
next push to `main`.

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
