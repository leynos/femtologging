# Make HTTP and socket handler shutdown deadline-bounded and cancellation-aware

This ExecPlan is a living document. The sections `Constraints`,
`Tolerances (exception triggers)`, `Risks`, `Progress`,
`Surprises & discoveries`, `Decision log`, `Outcomes & retrospective`,
`Conformance basis` and `Verification plan` must be kept up to date as work
proceeds.

Status: IN PROGRESS

All four milestones are implemented. The current candidate is rebased onto
`origin/main` at `d234990f7f4c3a7c000215cf9b90ad87d5d43c25`; the five feature
commits replay unchanged. Retry 19 passed all deterministic gates, including
formatting, lint, typecheck, Rust and Python tests, Markdown lint and Mermaid
validation. Pytest reported 791 passed, 5 skipped and 1 warning, with 66
snapshots passed. CodeRabbit found two paths that silently dropped the
in-flight record when its shutdown budget expired. Both are fixed and covered
by worker tests. The follow-up ExecPlan update also passed its documentation
gates. Two later CodeRabbit requests were rate-limited and returned no review
findings; their cooldowns and retry history are recorded below. CodeRabbit then
completed a review on `cb8a81e` and identified five findings. The current
follow-up serializes two log-capture tests, flushes shutdown drops immediately,
and uses `Python::try_attach` in both destructors. The full formatting, lint,
typecheck and test gates have since passed; pytest reported 791 passed, 5
skipped and 1 warning, with 66 snapshots passed. A Markdown spelling error was
fixed, and the final formatting, Markdown lint and Mermaid gates passed. The
draft PR awaits a fresh CodeRabbit review of the follow-up.

Related: [Issue #436](https://github.com/leynos/femtologging/issues/436).

## Purpose / big picture

`FemtoHTTPHandler` and `FemtoSocketHandler` each own a worker thread that
forwards records to a remote collector. When the handler is closed, the worker
is asked to stop and then joined. Both handlers had two defects in that path,
and they are the same two defects in both.

The first is that the shutdown request is carried on the same bounded channel
as the records. That channel is exactly where the backlog lives, so a shutdown
queued behind a thousand records is not read until the record in hand has
exhausted its retries. Worse, the send that queues it blocks when the channel
is full, and the acknowledgement wait that follows used a timeout whose failure
was discarded (`let _ = ack_rx.recv_timeout(...)`) before an unconditional
join. A worker too busy to acknowledge therefore held the caller for its whole
backoff deadline, a figure `write_timeout` does not describe and the caller
never asked for. `close` runs from `Drop`, so that wait landed on whatever was
tearing the handler down, including interpreter shutdown.

The second is that nothing on the worker's retry path is interruptible. A retry
delay was `thread::sleep`, and the network calls inside an attempt used the
configured connect and write timeouts outright, with no awareness that a
shutdown was under way. A close arriving during a delay waited it out; a close
arriving during a request waited for the request.

After this change a caller can bound the cost of a close, and can rely on a
close meaning something. Concretely:

```python
handler = (
    femtologging
    .SocketHandlerBuilder()
    .with_tcp("collector.internal", 9020)
    .with_write_timeout_ms(200)
    .with_backoff(femtologging.BackoffConfig({"deadline_ms": 10_000}))
    .build()
)
# ... records are logged against a collector that has stopped answering ...
started = time.monotonic()
handler.close()
assert time.monotonic() - started < 0.7  # bounded by write_timeout, not by the
# ten-second backoff deadline
```

and the same for `FemtoHTTPHandler`. What a close does *not* promise is
delivery of the backlog: records still queued when the deadline passes are
discarded, and the count is warned about. That is the deliberate half of the
contract, and the alternative - draining an unbounded queue one connect timeout
per record - is the unbounded close this change removes.

## Orientation for a newcomer

This is the set of files the change touches, and what each is for.

`rust_extension/src/shutdown_signal.rs` is new, and is the centre of the
change. It holds `ShutdownSignal`, an `Arc`-shared phase and condition variable
that a handler and its worker both hold. Phases are `Running`,
`Draining { deadline }` and `Aborted`. The handler moves the phase; the worker
waits for it. Its `wait(delay)` is the interruptible stand-in for
`thread::sleep`, and its `clamp(configured_timeout)` bounds a network timeout
by what is left of the drain budget.
`rust_extension/src/shutdown_signal_tests.rs` holds its unit tests.

`rust_extension/src/sync.rs` is the concurrency seam: every worker on the
logging path spawns, communicates and locks through it rather than naming
`std::thread`, `crossbeam_channel` or `parking_lot` directly, because under
`--cfg loom` those names must resolve to Loom's primitives instead. That seam
previously re-exported `parking_lot::Condvar`, whose API is nothing like
Loom's, so it could not carry `ShutdownSignal` unchanged.
`sync/parking_condvar.rs` is new and wraps the ordinary `parking_lot::Condvar`
behind the calling convention `sync/loom_lock.rs` implements, so
`ShutdownSignal` names one `Condvar` and one `wait_timeout` in both
configurations.

`rust_extension/src/http_handler/worker.rs` and
`rust_extension/src/socket_handler/worker.rs` hold the two workers. Each keeps
its command channel for records, flush and shutdown, and now also holds the
`Arc<ShutdownSignal>`. Their retry sleeps go through `signal.wait`, and their
network timeouts through `signal.clamp`. Each has a `drain_pending` that runs
after a shutdown: every queued command gets at most one attempt, and whatever
is left when the deadline passes is counted and discarded.

`rust_extension/src/http_handler/handler.rs` and
`rust_extension/src/socket_handler/handler.rs` hold the two `close` methods.
Each now raises the signal *before* sending the shutdown command - so the
signal, not the queued command, is what wakes a sleeping worker - sends with
`send_deadline`, reads the acknowledgement with `recv_deadline`, and joins only
when the worker answered. Its Python `close` and `flush` run under `py.detach`.

`rust_extension/src/http_handler/queue.rs` and `response.rs` are new, split out
of the HTTP worker to keep it under the 400-line file limit: the command enum
with the drop/discard helpers, and the response classifier.

The tests are
`rust_extension/src/http_handler/{close_tests,drain_tests,retry_tests}.rs`,
`rust_extension/src/socket_handler/{close_tests,drain_tests}.rs`,
`rust_extension/src/shutdown_signal_tests.rs`, and the Python-level
`tests/test_http_handler.py` and `tests/test_socket_handler_close.py`.

`docs/users-guide.md` documents the close contract for both handlers, which is
where a user meets it.

## The central design problem

A blocking socket call cannot be cancelled. The standard library offers no way
to interrupt a `connect` or a `write` that is already in the kernel, and neither
`ureq` nor `native-tls` does. What a bounded close can control is therefore
not what a worker is doing, but three other things:

- whether the worker is *waiting* anywhere it could be woken - the retry sleeps,
  and the blocking receive on its command channel;
- how long any operation *started after the shutdown began* may run;
- whether the caller waits for the worker at all.

The design takes all three. The signal wakes the sleeps; `clamp` bounds the
operations; and `close` declines to join a worker that has not answered.

The deadline therefore bounds termination by *budget plus one network call
already in flight when the deadline fell*. That residual is not a gap in the
implementation, it is the shape of the problem: a worker blocked in `write` to
a peer that has stopped reading will not return until its write timeout
expires, and no amount of signalling changes that. The contract the code
documents says so, and `close` handles it by not waiting.

The second problem is a choice about what a close *means*. Once the deadline
passes, processing the queue is the one thing that cannot be allowed, because
each record left in it costs a connect timeout. So a shutdown is either a drain
(every queued record attempted once, within the budget) or an abort, and the
two are decided by the clock rather than by the queue's depth. Records that
miss the deadline are dropped with a count, not retried past it.

A third problem is the seam. `ShutdownSignal` needs a condition variable, and
the two halves of the seam disagree about the API: `parking_lot::Condvar` waits
with `wait_for(&mut guard, dur)` and returns nothing, while Loom's
`Condvar::wait_timeout(guard_by_value, dur)` returns a `LockResult` and a
`WaitTimeoutResult`. A single calling convention has to be chosen and one arm
adapted to it. The ordinary arm was adapted, because Loom's form is the one
that makes the reacquisition explicit and because adapting Loom would mean
suppressing its poison result.

## Constraints

- No public API changes. Both handlers keep their type paths, and the builder
  methods keep their names and signatures; the change is to what `close` does,
  not to how it is spelled.
- The ordinary build gains no new dependency. The signal is built from
  `parking_lot`, `crossbeam_channel` and `std`, all already present.
- The `--cfg loom` lane keeps working. Every new module on the logging path
  names its primitives through `crate::sync` rather than the crates directly,
  and the loom arm must not initialize a Python interpreter.
- No file exceeds 400 lines, per `AGENTS.md`.
- Clippy warnings are denied, and no lint may be silenced where a rewrite
  removes it.
- Every commit leaves the repository green on `make fmt`, `make lint`,
  `make typecheck`, `make test`, `make markdownlint` and `make nixie`.

## Tolerances (exception triggers)

- Scope: if the change requires editing more than twenty files under
  `rust_extension/src/` or `tests/`, stop and escalate.
- Interface: if a public signature must change - rather than a behaviour behind
  it - stop and escalate.
- Dependencies: if any new runtime or dev dependency is required, stop and
  escalate. The design above needs none.
- Timing: if the close contracts cannot be made robust against a loaded host
  without a wall-clock allowance more than an order of magnitude above the
  budget under test, stop and escalate: such a test would be measuring the host.
- Loom: if the seam's condition-variable change makes any existing Loom model
  fail, stop and escalate rather than changing the model's assertion.

## Risks

- Risk: discarding queued records on a timed-out close loses logs a user
  expected to be delivered.
  - Severity: medium. Likelihood: high, by design.
  - Mitigation: the discard is warned about with a count, and
    `docs/users-guide.md` states the rule where the user meets `close`. The
    alternative - an unbounded drain - is the defect this change fixes, so the
    choice is between a bounded close with a documented loss and an unbounded
    close with none.
- Risk: the abandonment path leaves a worker thread running past `close`.
  - Severity: medium. Likelihood: medium.
  - Mitigation: the signal is moved to `Aborted` before the handle is dropped,
    so the abandoned worker stops at the first check rather than draining; it
    ends when its in-flight request returns.
- Risk: the close contracts are timing tests, and timing tests fail on loaded
  hosts.
  - Severity: high. Likelihood: medium.
  - Mitigation: every budget asserted allows the configured 200ms plus 500ms
    for scheduling, still more than an order of magnitude below the 10s busy
    window. Tests wait on events (`arrived`, `accepted`, notification
    channels) rather than sleeping to reach a state.
- Risk: the `#[cfg(test)]`-only `notify_on_wait` hook is production code that
  exists for tests.
  - Severity: low. Likelihood: low.
  - Mitigation: it is `#[cfg(test)]`, so it is absent from every shipped build;
    and its justification - that a sleep long enough to guess a worker is in a
    delay measures the host, not the handler - is recorded at the field.

## Milestones

### EP-M1 - the shutdown signal

Deliver `shutdown_signal.rs`, its unit tests, and the seam change that lets it
exist in both configurations.

Acceptance: `ShutdownSignal` compiles in both arms; `wait` returns
`ShutdownRequested` when a shutdown begins during a delay, `Aborted` past the
deadline or after `abort`, and `Elapsed` when the delay runs out; `clamp`
returns `None` once the budget is spent rather than a zero duration.

### EP-M2 - wire the signal into both workers

Each worker takes an `Arc<ShutdownSignal>`, sleeps through `wait`, clamps its
timeouts through `clamp`, and gains a `drain_pending` that runs after a
shutdown.

Acceptance: a worker that is sleeping in a backoff wakes when the signal is
raised; no attempt begins after the deadline; queued records past the deadline
are counted and discarded.

### EP-M3 - rewrite `close` for both handlers, release the GIL, update docs

`request_shutdown` raises the signal, sends with `send_deadline`, reads with
`recv_deadline`, and reports `Finished` or `TimedOut` - the distinction an
earlier `let _ =` had thrown away. `close` joins only on `Finished`. Python
`close` and `flush` run under `py.detach`. `docs/users-guide.md` states the
drain-versus-abort semantics for both handlers.

Acceptance: `close` returns within its budget when the worker never
acknowledges, when the queue is full, and when the worker is mid-request; it
still joins a worker that does answer; and it is not `close` that holds the GIL.

### EP-M4 - tests

Rust: perpetual 429/5xx, stalled requests, full queue, close during backoff for
HTTP; failed connect and write, full queue, pending records, close during
backoff for the socket handler; and the wall-clock bounds asserted with a
scheduling margin. Python: the same close path through the binding, plus the
GIL assertion that Rust cannot make.

Acceptance: each contract is proved by reverting the fix and watching it fail,
and each has a companion asserting the fix is not a blunt unconditional
abandonment.

## Verification plan

The deterministic gates are the ordinary ones, and are run through `scrutineer`
so their output stays out of the planning context:

```shell
make fmt
make lint
make typecheck
make test
make markdownlint
make nixie
```

The Loom lane is checked as it is checked in CI, because the seam change
touches it:

```shell
RUSTFLAGS="--cfg loom" cargo clippy --manifest-path rust_extension/Cargo.toml \
  --no-default-features --all-targets -- -D warnings
RUSTFLAGS="--cfg loom" cargo test --release --manifest-path rust_extension/Cargo.toml \
  --no-default-features --test heavy
```

The Python-level close contracts run with the other Python tests:

```shell
uv run pytest tests/test_http_handler.py tests/test_socket_handler_close.py -v
```

Axioms. A blocking socket call is not cancellable, and neither `std` nor `ureq`
offers one; the deadline therefore bounds what is *started* after a shutdown,
plus one call already in flight. `crossbeam_channel`'s `send_deadline` and
`recv_deadline` measure their own timeouts against a supplied `Instant`.
`parking_lot`'s `Condvar` notifies without a lost-wakeup window when the
notification is sent while holding the guard, which is the discipline
`ShutdownSignal` uses.

Residual gap, named rather than hidden: an abandoned worker outlives `close` by
up to one network timeout. It is not joined, so nothing observes its exit, and
a test cannot assert it has gone without reintroducing the unbounded wait the
change removes. The durable guarantee is that it starts nothing further.

## Conformance basis

The upstream artefact is issue #436, whose "Expected behaviour" and "Acceptance
criteria" sections are the requirement set. There is no separate
terms-of-reference or technical-design document for this work, and none is
invented here.

Trace links, from issue #436 to milestone:

- "Separate cancellation signal / prioritized control channel observable during
  retry/backoff and network operations" maps to EP-M1 and EP-M2.
- "Replace blocking shutdown `send()` with a deadline-aware operation" and "Do
  not ignore acknowledgement timeout failure" map to EP-M3.
- "Define explicit graceful-drain vs abort semantics; do not process an
  unbounded backlog after the shutdown deadline" and "Make retry sleeps
  interruptible" map to EP-M2.
- "Total `close()` bounded even if the worker does not terminate cooperatively;
  no unconditional post-timeout join" and "Release the Python GIL around any
  blocking close/wait path" map to EP-M3.
- The four HTTP test criteria and the four socket test criteria map to EP-M4.
- "Assert wall-clock upper bounds with a reasonable scheduling margin" maps to
  the constants in EP-M4's tests, each of which allows the measured budget plus
  500ms for scheduling.

## Surprises & discoveries

- (2026-10-16) `parking_lot` 0.12.5 has no `Condvar::wait_timeout`. Its API is
  `wait`, `wait_until`, `wait_for(&mut guard, dur) -> ()`, `wait_while`,
  `wait_while_until` and `wait_while_for`. Loom 0.7.2 *does* have
  `wait_timeout`, but takes the guard by value and returns a `LockResult`
  wrapping the guard and a `WaitTimeoutResult`. The two arms of the seam
  therefore had to be normalized before `ShutdownSignal` could name either. See
  `Decision log`.
- (2026-10-16) Rustdoc is a usable source of crate-internal compile signal when
  the build admission pool is saturated. Invoking `rustdoc` with
  `--edition 2024`, `--crate-name femto`, `--crate-type lib` and
  `--output-format html` on `rust_extension/src/lib.rs` resolves the crate's
  own modules without touching `RUSTC_WRAPPER` or taking a compile slot;
  unresolved external crates are the only residual errors, and they are
  distinguishable by name. This caught a real break - `http_handler/handler.rs`
  still importing `enqueue_record` and `flush_queue` from `worker` after they
  moved to `queue` - that had been invisible for the whole session because no
  compilation had completed.
- (2026-10-16) The file-handler test support's `LOGS` static is process-global,
  shared across the whole test binary. Any test asserting on it must use the
  bare `#[serial_test::serial]` key, not a key named for its own module: a
  private key does not serialize against the file handler's tests, which then
  drain the buffer mid-assertion.
- (2026-10-16) Build admission has been saturated throughout: the status tool
  reports capacity 4 with 3 or 4 held and queues of well over a hundred crates,
  and a `cargo check` given forty minutes never obtained a slot. This plan's
  Rust milestones are therefore *implemented but not yet compiled*, and no Rust
  evidence is claimed for them. The pool is shared by every agent, repository
  and worktree on the machine, and several other `femtologging` worktrees were
  compiling concurrently, so this is contention rather than a fault.
- (2026-10-16) Ruff's error count is a poor guide to what is actually wrong.
  "Found 11 errors" was six distinct rule codes over ten source locations, and
  `ruff check --statistics` prints that shape in six lines where the annotated
  form needs 165. Four of the eleven were one rule (`any-type`) at four places,
  and one (`pytest-assert-in-except`) was emitted twice for an identical span,
  `tests/test_socket_handler_close.py:315:13` - so neither the count nor a
  count of headings maps onto the work. Only 3 of the 11 were auto-fixable
  (`unsorted-imports` twice, `unused-noqa` once), and `ruff format --check` was
  clean throughout, because import-block ordering is a `check` rule and not a
  formatting one. The remaining 8 needed hand edits; assuming a formatting pass
  makes `make lint` clean would have wasted a cycle.
- (2026-10-23) The versioned ureq 2.12.1 `AgentBuilder` documentation confirms
  that `.timeout()` bounds the overall request, including DNS, connection,
  redirects and response reading, but slow DNS may exceed it because DNS
  resolution cannot be interrupted. `.timeout_connect()` takes precedence over
  `.timeout()` for connection setup. This supports the existing boundary: close
  stops waiting at its deadline and detaches a worker still inside an in-flight
  request; DNS resolution remains outside cooperative cancellation.
- (2026-10-23) The first post-rebase full gate run found a Whitaker complexity
  finding in `connect_transport`; extracting the TCP/TLS and Unix branches into
  helpers removed the nested condition clusters without a suppression. The
  following test run exposed setup assumptions: immediate 503 responses and
  refused connects let the backlog finish before close, and a 1 MiB frame did
  not reliably fill this host's loopback send buffer. The regression fixtures
  now coordinate on a stalled HTTP response or TLS ClientHello, and wait on the
  log capture condition variable for detached workers to report discarded
  records. A shutdown-signal test now expects an immediate result when a wait
  begins after shutdown has already started.
- (2026-10-23) Self-review found a race in both worker drains: the worker can
  observe the separate shutdown signal before the queued shutdown command, then
  acknowledge that command while it is still processing later records. That
  made the acknowledgement an unreliable barrier for the handler's join. Both
  drains now retain queued shutdown senders until after their remaining work;
  the socket does its final silent flush first. Focused tests queue a shutdown
  before a record held in HTTP I/O or a TLS handshake and assert that no
  acknowledgement is available until the record completes.

## Decision log

- Decision: normalize the seam's condition variable on Loom's calling
  convention, and adapt `parking_lot` to it, rather than the reverse.
  - Rationale: Loom's form makes the reacquisition explicit, which is the part
    a reader is most likely to get wrong; and adapting Loom would have meant
    discarding its poison result, which is information the ordinary arm does not
    have and should not pretend to. `sync/parking_condvar.rs` therefore consumes
    the guard and returns nothing, and `sync/loom_lock.rs` keeps Loom's shape.
  - Date: 2026-10-16.
- Decision: `clamp` returns `Option<Duration>`, with `None` meaning "do not
  begin this operation", rather than a zero duration.
  - Rationale: both `std` and `ureq` read a zero duration as *no timeout* rather
    than *no time*, which is the opposite of what a spent budget is asking for.
  - Date: 2026-10-16.
- Decision: a timed-out close abandons the worker rather than joining it, and
  moves the signal to `Aborted` before dropping the handle.
  - Rationale: joining is the unbounded wait being removed. The abort matters
    separately: the worker holds the only other reference to the signal, so one
    left in `Draining` would keep telling it that budget remained.
  - Date: 2026-10-16.
- Decision: split the HTTP worker into `queue.rs` and `response.rs`, and the
  socket close tests into `close_tests.rs` and `drain_tests.rs`.
  - Rationale: `AGENTS.md` caps a code file at 400 lines. The split is along the
    same axis in both cases - fixtures and helpers for the close path separate
    from those for the retry path - so it also groups by concern rather than
    only by size.
  - Date: 2026-10-16.
- Decision: declare `shutdown_signal` as a private module (`mod`, not
  `pub mod`), and fence its doc examples `ignore` as `sync.rs` does.
  - Rationale: the signal is internal machinery between a handler and its own
    worker, and no caller outside the crate can reach a `ShutdownSignal`. A
    `pub mod` would have widened the crate's supported Rust surface - the
    opposite of this change's no-public-API constraint - to expose a type whose
    only constructor pair is called from two `close` methods. The other private
    modules that carry examples (`sync.rs`, the two handlers) fence them
    `ignore` for the same reason: a doctest is a promise to a caller, and there
    is no caller. `examples/shutdown_signal` and a `pub use` were considered and
    rejected for the same reason.
  - Date: 2026-10-16.
- Decision: keep the `#[cfg(test)]`-only `notify_on_wait` hook on
  `ShutdownSignal`.
  - Rationale: the alternative is a test that sleeps long enough to expect the
    worker to be in a delay, which measures the host rather than the handler.
    The hook is absent from every shipped build.
  - Date: 2026-10-16.

## Progress

- 2026-10-16: EP-M1 delivered. `shutdown_signal.rs` (294 lines),
  `shutdown_signal_tests.rs` (299 lines, 13 tests), `sync/parking_condvar.rs`
  (50 lines) and the `loom_lock.rs` normalization are in place.
- 2026-10-16: EP-M2 delivered. Both workers take the signal, sleep through
  `wait`, clamp through `clamp`, and drain-then-discard.
- 2026-10-16: EP-M3 delivered. Both `close` methods now raise the signal first,
  send with `send_deadline`, read with `recv_deadline`, and join only on
  `Finished`. Both Python `close` and `flush` run under `py.detach`.
  `docs/users-guide.md` updated for both handlers.
- 2026-10-16: EP-M4 delivered. Rust close, drain and retry tests for both
  handlers; Python close, GIL, backlog and delivery tests.
- 2026-10-16: first gate run, delegated to `scrutineer`. `make check-fmt`
  passed. `make lint` failed, but only in its first stage: Ruff raised 11 errors
  (`lint-python`'s later stages - `interrogate`, Pylint, `df12-python-lints`,
  `ambrleaks`, Skylos - never ran), and the failure aborted the target before
  `lint-rust` was reached. All 11 were in the two new Python test files and are
  now fixed: import-block ordering in both; an unused `# noqa: N802` and a
  `typ.Any` return removed from `tests/test_http_handler.py`, whose
  `log_message` override now takes the `(format: str, *arguments: object)`
  signature `tests/test_native_handler_context.py` already uses, and whose
  `server.arrived` is reached through `typ.cast` rather than a
  `type: ignore[attr-defined]`; `tests/test_socket_handler_close.py` gained
  `SocketHandlerBuilder` and `FemtoSocketHandler` return types, a `Returns`
  section, and a `_handle_expecting_refusal` helper so the queue-full assertion
  sits outside the `except` block. Ruff now reports "All checks passed!" and
  `ruff format --check` reports 184 files already formatted. `make lint-python`
  must be re-run in full, because four of its stages have still never seen
  these files.
- 2026-10-16: the compile pool remains saturated (`capacity 4, held 3`-`4`) with
  queues of a hundred or more crates. `make typecheck` has been running since
  17:44 and is still compiling dependencies at 18:06; its log has not advanced
  past `Building femtologging`, because the build is admitted crate by crate
  and the crate itself is last. This is queueing, not a hang:
  `build-limits-status` reports the supervisor healthy, and artefacts appear in
  this worktree's `target/release/deps/` on every poll. No Rust gate has
  completed yet, and a `cargo check` given forty minutes earlier never obtained
  a slot either.
- 2026-10-16: the wait was spent on a read-only review of every file this change
  adds, since none of them has been compiled. What it established: the four
  never-compiled test files resolve every helper through `super` or an absolute
  `crate::` path that exists (`CapturedRequest`, `spawn_retry_server`,
  `tcp_listener`, `send_info_record` in `tests.rs`; `build_http_handler`,
  `spawn_mock_server` likewise; `handlers::file::test_support` is
  `#[cfg(test)]` and so is reachable from `#[cfg(test)]` test modules), the
  config structs they construct derive `Clone` and implement `Default` for their
  `..Default::default()` tails, and every import they add is used -
  `HandlerBuilderTrait` in `socket_handler/close_tests.rs` is needed for
  `build_inner`, which only the trait provides. The `#[rstest]` +
  `#[serial_test::serial]` attribute pairing they use already appears elsewhere
  in the crate, and `serial_test` 4 is a declared dev-dependency. No defect was
  found by inspection; the compiler has still to confirm it.
- 2026-10-16 18:27: **the change compiles.** `make typecheck`'s `build`
  dependency completed a
  `cargo rustc --features test-util --lib --crate-type cdylib` for this crate
  and produced `target/release/deps/lib_femtologging_rs.so`, after the whole
  dependency graph had been admitted and compiled one crate at a time. The
  compile is the first evidence this never-compiled change needed, and it is
  clean: the log carries no `error[E...]` diagnostic and not one mention of a
  path under `rust_extension/src`. Every production source this change adds or
  alters is listed in the artefact's dependency file - `shutdown_signal.rs`,
  `sync/parking_condvar.rs`, `http_handler/{handler,worker,queue,response}.rs`,
  `socket_handler/{handler,worker,transport}.rs`, `sync.rs` and `mod.rs`. What
  the compile did *not* cover is the test modules (`close_tests.rs`,
  `drain_tests.rs`, `retry_tests.rs`, `response_classification_tests.rs`,
  `shutdown_signal_tests.rs`, `sync_tests.rs`) and the Loom arm of `sync.rs`,
  neither of which a `--lib` cdylib build reaches. `make test` and the Loom
  lane are what compile those.
- 2026-10-16 19:31: **the compile pool failed, and the gates are blocked on
  it.** `buildlimits-supervisor.service` stopped and its automatic restart
  refused to complete. The sequence, from the journal: the old supervisor
  entered `stopping` and drained every compiler group before exiting; systemd
  restarted it two seconds later; and the new instance exited `status=2` with
  `start-up requires operator action: startup cannot account for managed cache
  work: live sccache service may retain detached requests; visible compiler
  roots=0; operator accounting required; operator action: coordinate a quiet
  sccache stop, verify its empty cgroup, then restart admission; do not
  interrupt active builds`.
  The unit sets `RestartPreventExitStatus=2` and `Restart=always`, so exit 2 -
  by design - stops the restart loop and waits for a person.
  `buildlimits-supervisor.service` is now `failed`, the socket at
  `/run/user/1000/buildlimits/supervisor.sock` is stale (mtime 2026-10-12) and
  refuses connections, and `build-limits-status` cannot reach it. The managed
  `buildlimits-sccache.service` is still `active (running)`.
- 2026-10-16: this is what failed `make typecheck`, and it failed it without
  any code being at fault. `maturin develop` reached its
  `--features python,test-util` compile of the debug profile, queued the
  dependency graph, and then every queued crate was answered with the refusal
  quoted above; the log carries nine of them. Cargo reported the consequence
  the only way it can, as `error: could not compile` for each named crate, and
  each of those errors carries a `Caused by` line naming the wrapper rather
  than a diagnostic of its own. The failing process is `build-limits-rustc`
  every time, never `rustc`. Nine crates were reported this way, all of them
  dependencies from crates.io: `pyo3`, `ordered-multimap`, `tempfile`,
  `tracing-subscriber`, `chrono`, `tracing`, `serde_json`, `encoding_rs` and
  `icu_provider`. `make` then reported a failure at `Makefile:97`, the `build`
  target. Per the standing rule the remedy is not to work around admission: the
  supervisor needs the operator action its own message names. Everything from
  `make typecheck` onward is blocked behind it, because each of those gates
  compiles: `make test`, the Loom lanes, and the four `make lint-python` stages
  that have still never run.
- 2026-10-16 checkpoint: CodeRabbit review, push and draft pull request had not
  started. Deterministic gates had priority over review.
- 2026-10-16: `make lint-python` run in full for the first time, and green -
  all six stages. Two things had to happen first. Pylint's `C0302` had to be
  answered: `tests/test_socket_handler_close.py` was 409 lines against the
  project's 400-line limit, which is both a `pyproject.toml` setting
  (`max-module-lines = 400`, with `too-many-lines` deliberately enabled to
  "flag modules that have grown large enough to deserve splitting") and an
  AGENTS.md rule, so the file was split rather than the message suppressed. The
  shared harness - the stalling `socketserver` pair, `stall_the_worker`,
  `build_handler`, `unused_port`, `recv_exactly` and the timing constants -
  moved to `tests/socket_handler_support.py`, following the flat
  `tests/logger_support.py` convention; the test module keeps the fixture and
  the seven tests. 283 and 165 lines now. And the compile supervisor had to
  come back: `build-limits-status` reports
  `capacity 4, held 3, healthy, caching through sccache`, so the 19:31 outage
  cleared without intervention. With both in place, Ruff, `interrogate` (100%),
  Pylint (10.00/10), `df12-python-lints` (10.00/10), `ambrleaks` and
  `skylos --gate` all passed - the last three for the first time.
- 2026-10-16: second gate run, delegated to `scrutineer`. Four gates passed:
  `make check-fmt`, `make markdownlint`, `make nixie` and
  `make loom-runner-test` (14 tests). Three did not, and two of the three were
  real defects that nothing before this run could have found, because both live
  in code that a `--lib` cdylib build never reaches.
  - `make lint` failed in `lint-env-policy` with `E0061`: the new `signal`
    parameter on `connect_transport` had not been added to its call site in
    `socket_handler/tests.rs`. `lint-python` and `lint-lanes-test` had already
    passed in the same run.
  - `make test` failed in its `lint-rust-clippy` prerequisite on an unused
    `Duration` import in `http_handler/worker.rs`, promoted to an error by
    `-D warnings`. Not one test executed in any feature lane, and `pytest`
    never ran: the new HTTP and socket tests are still entirely unexercised.
  - `make typecheck` was blocked by a transient admission refusal in
    `maturin develop`'s host-target probe. `build-limits-status` reports the
    supervisor healthy now, so this was a passing refusal, not a recurrence of
    the 19:31 failure.
- 2026-10-16: both defects fixed. `socket_handler/tests.rs` now builds a
  running `ShutdownSignal` beside the transport and passes it to
  `connect_transport`, with a comment saying why the signal is deliberately
  inert there - the test is about the configured timeout, not the signal - and
  `http_handler/worker.rs` imports only `Instant`. `connect_transport` has two
  call sites and both are corrected; no other signature this change alters has
  a stale caller.
- 2026-10-23: refreshed the principal remote's `main` to
  `da1d82e8530ea62415d0d6f97078ed945b76b69b`. The feature series was linear from
  `27ea9feffb1452f626531f75024f2e20e7484b0c`, with no merge commits, and the
  branch had no remote ref or stack membership. Preserved old head, base, and
  target under `refs/codex/recovery/issue-436-20261023/`. Rebased the feature
  series with Git's text merge driver because Weave was selected only by
  host-global attributes. `range-diff` matched the four implementation commits
  and this plan; the spelling-config-only commit was dropped because current
  `main` already contains the regenerated, narrower policy. The current guide
  now states the one-attempt drain, detach-after-deadline, and GIL-release
  behaviour. `make check-fmt`, `make markdownlint`, and `make nixie` passed for
  the documentation update; full candidate gates remain pending.
- 2026-10-23: the first full post-rebase candidate run passed `make fmt`,
  `make check-fmt`, `make lint` and `make typecheck`. `make test` passed its
  Python and Loom stages, then reported 323 passed, 1 failed and 1 ignored in
  the no-default-features Rust lane. The sole failure was the HTTP full-queue
  close test expecting an abandonment warning. Its log showed the request
  timing out, the worker observing shutdown and processing the queued record,
  then acknowledging before the deadline; the test had required detachment even
  though this cooperative completion is valid. The assertion is now split: that
  full-queue case checks the deadline bound, while the non-acknowledging case
  retains the abandonment assertion. The healthy-close case now verifies that
  both queued records reached the collector, and close-time assertions allow
  the configured 200ms budget plus 500ms for scheduling. The corrected
  candidate passed the full sequence: `make fmt`, `make check-fmt`, `make lint`,
  `make typecheck`, and `make test` (763 passed, 5 skipped, 1 warning),
  followed by `make markdownlint` and `make nixie`. Formatting made no tracked
  changes.
- 2026-10-23: a later self-review found that a queued `Shutdown` command could
  be acknowledged before the worker finished draining later records. The
  acknowledgement now follows the full drain, discard warning and socket final
  flush. New worker-level tests hold a request or TLS handshake open behind a
  queued shutdown command to verify the acknowledgement stays pending. The full
  code gate sequence must be rerun before CodeRabbit review. The first rerun
  stopped at `make lint`: the socket worker test lacked imports for its three
  transport types. The test import is corrected; no later gate ran in that
  attempt. A subsequent run stopped with `No space left on device` while
  writing the Rust incremental target and `.uv-cache` temporary files. The
  `make clean` target removed 17.3GiB of generated Rust artefacts; the
  filesystem now has 50GiB free. The retry-path bounds now allow the configured
  200ms plus 500ms, and the backoff tests explicitly assert that closing does
  not start another retry. The post-turn checks will rerun the requested gates.
- 2026-10-24: rebased the five-commit feature series onto
  `5377ceb482a7ad57dca7a8d7b67abeb0f15a560c`, which adds private Rust item
  documentation and expands lint coverage. Restored the uncommitted
  acknowledgement correction and tests. The HTTP retry-budget test now covers
  both perpetual 429 and 503 responses, the socket capacity-one test asserts
  `QueueFull` explicitly, and the pending-record close test checks the
  write-timeout budget plus 500ms. No final gate has run on the rebased
  candidate yet.
- 2026-10-24: the scope tolerance was exceeded: the candidate changes 25 files
  under `rust_extension/src` and 3 under `tests/`, above the plan's 20-file
  limit. The user explicitly directed continuation until a draft PR or an
  intractable impasse, requested the push and draft PR, and removed any
  wall-clock limit. That direction supersedes this plan's stop trigger for this
  issue. The cross-cutting files are required to implement and test the agreed
  shutdown contract across both handlers, the shared signal, socket transport,
  Python bindings and user documentation. Continue through final gates, review,
  commit, push and draft PR.

- 2026-10-24: CodeRabbit completed after the deterministic gates passed and
  reported six minor findings: a stale close-time assertion comment, grammar,
  HTTP abandonment wording, a Markdown sentence rendered as a list, an
  inaccurate scheduling-margin statement, and the scope decision record added
  above. All six findings were corrected and reviewed.
- 2026-10-24: the next full gate run reached the private-doc contract test after
  all Clippy lanes and test-support documentation checks passed. Its documented
  fixture timed out after 620 seconds while compiling the full feature matrix;
  shared compiler admission was healthy, but other repository builds held the
  queue. The documented fixture now has a 20-minute timeout and the expected-
  failure fixture retains 10 minutes, with a 20-second margin for the combined
  test. This allows the successful feature-matrix run to complete without
  relaxing the lint assertion. Rerun all gates before publication.
- 2026-10-25: the sequential candidate run passed formatting, lint, typecheck,
  and the no-default-feature Rust suite. The Python-feature Rust suite found
  that a stalled request can finish cooperatively at the shutdown deadline;
  this test first observed the worker's counted-drop warning. The separate
  full-queue case continues to cover bounded command insertion. The first rerun
  then failed in lint's strict second-pass Rust lane because the old
  abandonment assertion helper had become unused. After removing that helper,
  the next run passed lint and typecheck, but the same stalled-request test
  observed the deadline-triggered detach warning before the worker's eventual
  drop warning. The request and acknowledgement race at the same deadline, so
  the test now accepts either report while asserting the close bound and no
  retry after the request timeout. Rerun the full gate sequence before review.
- 2026-10-25: the next post-turn gate attempt again ran out of filesystem
  space. Trybuild could not create a temporary file under
  `rust_extension/target`, Pylint failed while importing from its cached
  environment, and Nixie could not create a temporary directory. Removed only
  this worktree's generated Cargo target with `cargo clean`; this reclaimed
  16.5GiB and left 109GiB free. No source files changed. Rerun the full gates
  from this clean target before review.
- 2026-10-25: after the clean-target gates and the 15-minute review observation,
  CodeRabbit reported five minor findings: two Oxendict spelling corrections, a
  file-level Ruff assertion suppression, and two related races in the HTTP
  retry-delay test. Removed the suppression in favour of `pytest.fail`,
  corrected the comments, and made the HTTP test pause the worker until close
  publishes shutdown before releasing it. Its backoff is five seconds so an
  uninterruptible wait also has a clear wall-clock failure. Rerun all
  deterministic gates before the final review and publication.
- 2026-10-25: retry 15 passed formatting, lint, and typecheck, then found the
  socket healthy-close test's default one-second acknowledgement budget could
  expire in the `log-compat` lane under host load, despite a preceding flush
  acknowledgement. Raised only that positive-path fixture's timeout to five
  seconds; the other socket tests retain their short budgets and explicit
  elapsed-time assertions. Rerun the full deterministic sequence before review.
- 2026-10-25: the final review found five more minor issues: two Loom-comment
  spelling errors, a Python backlog test that used refused connects instead of
  a stalled write, stale HTTP close-test comments, and rate-limited drops not
  being flushed before worker acknowledgement. The Python backlog now holds the
  worker in its flood write; the close-test comments match their assertions;
  `RateLimitedWarner::flush` emits pending counts despite the rate limit, and
  both workers flush those counts before their final acknowledgements. Rerun
  all deterministic gates before the next review.
- 2026-10-25: the next review found shutdown-budget early returns that could
  lose the in-flight HTTP or socket record without incrementing its drop
  counter. Both workers now count and warn when an expired budget refuses a
  request, connect, or write. Added serialized worker tests for the aborted
  HTTP request and expired socket connect and write budgets. The stalled TLS
  close test also asserts the in-flight record is reported as a shutdown drop
  when its connect budget expires. Run the complete deterministic gates before
  the next review.
- 2026-10-25: refreshed `origin/main` to
  `d234990f7f4c3a7c000215cf9b90ad87d5d43c25`. Saved the prior feature head and
  dirty worktree under `refs/codex/recovery/issue-436-20261025-*`, rebased the
  five-commit series, and restored the dirty changes. `git range-diff` shows
  all five commits unchanged; the merge base is now the refreshed
  `origin/main`. Rerun the full deterministic gates on this rebased candidate
  before review.
- 2026-10-25: retry 18 passed formatting, all lint lanes and typecheck. The
  no-default Rust suite passed (333 passed, 1 ignored), all six trybuild cases
  passed, and doctests passed (27 passed, 13 ignored). The Python-feature Rust
  suite found that the stalled TLS test waited for a later connect-error log
  after its shutdown-drop marker; the logger helper returns as soon as the
  marker arrives. The assertion now checks the marker itself, which is the
  shutdown-drop contract under test. Rerun the complete gate sequence.
- 2026-10-25: retry 19 passed `make fmt`, `make check-fmt`, `make lint`,
  `make typecheck`, `make test`, `make markdownlint` and `make nixie`. Python
  pytest reported 791 passed, 5 skipped and 1 warning; 66 snapshots passed. All
  Rust feature suites, six-case trybuild matrices and doctests passed. The
  final CodeRabbit request was rate-limited. The user requires a randomized
  45-90 minute foreground `vsleep` before retrying; the ExecPlan update also
  requires its documentation gates before that review.
- 2026-10-25: the ExecPlan status update passed `make fmt`, `make check-fmt`,
  `make markdownlint` and `make nixie`. The post-cooldown CodeRabbit request
  was rate-limited again with a reported wait of 2m58s and returned no
  findings. The user requires another randomized 45-90 minute foreground
  `vsleep` before a further review retry.
- 2026-10-25: a concurrent pytest plugin-registration change moved the
  `logging_context_steps` plugin into the root `tests/conftest.py`; a later
  full run reported 791 passed and 5 skipped without the existing
  `PytestAssertRewriteWarning`. That change was unrelated to #436 and is
  preserved outside this branch, so its warning-free result is not candidate
  evidence. The #436 candidate's full test result remains retry 19 above.
- 2026-10-25: CodeRabbit completed on `cb8a81e` after the required cooldown and
  found five issues. The two queued-shutdown worker tests needed serialization
  because they share process-wide log capture. Shutdown drops needed to use
  `record_drop` followed by `flush`, preserving the general rate-limit reset
  behaviour while reporting drops during close. PyO3 0.30.1 documents that
  `Python::attach` may panic during interpreter shutdown, so both destructors
  now use `Python::try_attach` and fall back to the Rust-only bounded close.
  Targeted regression tests cover the warning interval; rerun the complete
  deterministic gate sequence before requesting another review.
- 2026-10-26: The full Rust suites passed, but pytest exposed a GIL wait in the
  stream handler's Python `close()` and `flush()` methods when its worker runs
  a Python formatter. The close timeout let the formatter write after
  `capfd.readouterr()`. Both methods now detach while waiting, and `Drop` uses
  `Python::try_attach` before detaching, matching the HTTP and socket handler
  pattern. The users' guide documents the GIL behaviour. Rerun the full
  deterministic sequence after this fix.
- 2026-10-26: The follow-up passed formatting, lint, typecheck and the complete
  test suite. Pytest reported 791 passed, 5 skipped and 1 warning, with 66
  snapshots passed. The first Markdown lint run caught `behavior` in this
  history entry; it was corrected to `behaviour`. Formatting, Markdown lint and
  Mermaid validation then passed. A fresh CodeRabbit review remains.

## Outcomes & retrospective

The first corrected candidate passed every deterministic gate, including the
full Rust feature lanes, Loom tests, Python tests, formatting, lint, typecheck,
Markdown lint and Mermaid validation. An early test failure incorrectly
required a cooperative full-queue worker to be detached; that case now checks
the deadline without requiring detachment. A later self-review found a real
acknowledgement-ordering race that the existing handler-level tests did not
force. The new focused worker tests hold an in-flight request or TLS handshake
open behind a queued shutdown and verify that the acknowledgement remains
pending. The latest test run showed that the stalled HTTP request can either
finish near the deadline or still be in flight when close detaches the worker.
The test accepts either warning, checks the close bound, and waits beyond the
request timeout to ensure no retry begins. Retry 19 passed the full
deterministic gate sequence, and the follow-up status update passed its
documentation gates. Two later CodeRabbit requests were rate-limited and
returned no findings; the user-directed cooldowns are recorded in the decision
log.
