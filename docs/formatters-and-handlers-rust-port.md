# Porting Formatters and Handlers to Rust

This document outlines a safe and thread‑aware design for moving formatting and
handler components from Python to Rust. It complements the
[roadmap](./roadmap.md) and expands on the design ideas described in <!--
markdownlint-disable-next-line MD013 -->
[`rust-multithreaded-logging-framework-for-python-design.md`](./rust-multithreaded-logging-framework-for-python-design.md).

## Goals

- Provide Rust implementations of formatting and handler logic equivalent to
  CPython's `logging` module, keeping API familiarity.
- Ensure all components satisfy `Send`/`Sync`, so they can operate across
  threads without unsafe code.
- Maintain a producer–consumer model for handlers, so application threads are
  never blocked by I/O.
- Introduce a `FemtoLevel` enum, so loggers can efficiently filter messages
  before records reach handlers.
- Use `crossbeam-channel` as the initial MPSC queue, consistent with
  [`dependency-analysis.md`](./dependency-analysis.md).

`FemtoLogRecord` bundles these fields into a `RecordMetadata` struct. The
struct contains timestamp, source location, thread data and structured
key‑value pairs. Formatters read this metadata to produce fully detailed
messages.

## FemtoFormatter Trait

`FemtoFormatter` defines how a `FemtoLogRecord` becomes a string. The default
implementation is intentionally simple and follows a common "logger [LEVEL]
message" convention:

```rust
pub trait FemtoFormatter: Send + Sync {
    fn format(&self, record: &FemtoLogRecord) -> String;
}

pub struct DefaultFormatter;

impl FemtoFormatter for DefaultFormatter {
    fn format(&self, record: &FemtoLogRecord) -> String {
        format!("{} [{}] {}", record.logger, record.level, record.message)
    }
}
```

Formatters are `Send + Sync` so a handler thread can hold them without
synchronization. Custom formatters may store additional configuration, such as
timestamp layouts. Future extensions can include structured output with `serde`
when network handlers are introduced.

## FemtoHandler Trait and Implementations

Each handler owns an MPSC receiver and runs in a dedicated consumer thread.
Application code holds the sender cloned from the channel. Handlers implement a
common trait:

```rust
pub trait FemtoHandlerTrait: Send + Sync {
    fn handle(&self, record: FemtoLogRecord);
    fn flush(&self) -> bool { true }
}

/// Base Python-exposed class. Methods are no-ops by default.
#[pyclass(name = "FemtoHandler", subclass)]
pub struct FemtoHandler;
```

Implementations should forward the record to an internal queue with `try_send`
so the caller never blocks. If the queue is full, the record is silently
dropped and a warning is written to `stderr`. This favours throughput over
completeness: records may be lost to keep the application responsive. Advanced
use cases can specify an overflow policy when constructing a handler. Python
callers pass an overflow policy string literal ("drop", "block", or
"timeout:N") to the constructor, where N is the timeout in milliseconds (for
example, "timeout:500"). The policy may also be extended to support options
like back pressure, writing overflowed messages to a separate file, or emitting
metrics for monitoring purposes:

- **Drop** – current default; records are discarded when the queue is full.
- **Block** – the call blocks until space becomes available.
- **Timeout** – wait for a fixed duration before giving up and dropping the
  record.

Every handler provides a `flush()` method, so callers can force pending
messages to be written before shutdown.

```python
from femtologging import FemtoFileHandler

# Block until space is available
handler = FemtoFileHandler(
    "app.log", capacity=4096, flush_interval=1, policy="block"
)

# Or wait up to 250 ms when the queue is full
timeout_handler = FemtoFileHandler(
    "app.log", capacity=4096, flush_interval=10, policy="timeout:250",
)
```

Legacy constructors have been removed:

- ``with_capacity``
- ``with_capacity_blocking``
- ``with_capacity_timeout``
- ``with_capacity_flush``
- ``with_capacity_flush_blocking``
- ``with_capacity_flush_timeout``
- ``with_capacity_flush_policy``

The timeout behaviour is configured via the policy string as ``"timeout:N"``,
where ``N`` is the timeout in milliseconds and must be greater than zero.

Customise capacity, flush behaviour or overflow policy via keyword arguments on
the constructor.

The constructor enforces several invariants on the configuration:

- ``capacity`` and ``flush_interval`` must be greater than zero.
- ``policy`` must be ``"drop"``, ``"block"`` or ``"timeout:N"``.
- ``N`` must be greater than zero when specifying a timeout policy.

### StreamHandler

`FemtoStreamHandler` writes formatted records to `stdout` or `stderr`. The
consumer thread receives `FemtoLogRecord` values, moves the writer and
formatter into the worker thread, and writes directly without locking. This
mirrors the design in
[`concurrency-models-in-high-performance-logging.md`][cmhp-log]. The default
bounded queue size is 1024 records, but `FemtoStreamHandler::with_capacity`
lets callers configure a custom capacity when needed. Flushing is driven by a
timeout measured in milliseconds.

Dropping a handler closes its channel and waits briefly for the worker thread
to finish flushing. If the thread does not exit within the configured flush
timeout (one second by default), a warning is printed, and the drop continues,
preventing deadlocks during shutdown.

#### Sequence Diagram

```mermaid
sequenceDiagram
    participant Caller
    participant FemtoStreamHandler
    participant Channel
    participant WorkerThread
    participant Stream

    Caller->>FemtoStreamHandler: handle(FemtoLogRecord)
    FemtoStreamHandler->>Channel: try_send(record)
    Note right of Channel: (Non-blocking, async)
    WorkerThread-->>Channel: receive(record)
    WorkerThread->>Stream: format + write(record)
    WorkerThread->>Stream: flush()
```

### FileHandler

`FemtoFileHandler` behaves similarly but manages an owned file handle. Rotation
variants (`FemtoRotatingFileHandler`, `FemtoTimedRotatingFileHandler`) build on
this by performing rotation logic inside their consumer threads.

The Rust implementation resides under `rust_extension/src/handlers/file`. This
module is split into three pieces, so each concern stays focused:

1. `config.rs` — configuration structures and defaults.
2. `worker.rs` — the background writer thread and its helper types.
3. `mod.rs` — the public `FemtoFileHandler` API.

```rust
use femtologging_rs::handlers::file::FemtoFileHandler;
```

`FemtoFileHandler` exposes `flush()` and `close()` methods, so callers can
drain pending records and stop the background thread explicitly. Dropping the
handler still performs this cleanup if the methods aren't invoked.

By default, the file handler flushes the underlying file after every record to
maximise durability. To batch writes, pass a custom configuration via
`FemtoFileHandler::with_capacity_flush_policy()` (Rust) or set keyword
arguments on ``FemtoFileHandler`` (Python). Setting ``flush_interval`` defers
flushing until the specified number of records have been written. The value
must be greater than zero, so periodic flushing always occurs. Higher values
reduce syscall overhead in high-volume scenarios. Internally the handler
buffers writes with `BufWriter`, so records only reach the file once a flush
occurs or the handler shuts down.

The worker thread begins processing records as soon as the handler is created.
Production code therefore leaves the optional `start_barrier` field unset. Unit
tests may use this barrier to synchronize multiple workers, avoiding race
conditions. Should a future feature require coordinated startup (for example,
rotating several files at once), the `WorkerConfig` creation logic will need to
expose this.

Calling `flush()` sends a `Flush` command to the worker thread and then waits
on a dedicated acknowledgment channel for confirmation. The worker responds
after flushing its writer, giving the caller a deterministic way to ensure all
pending records are persisted. The wait is bounded by the handler's
`flush_timeout_secs` setting (one second by default) to avoid indefinite
blocking.

All handlers spawn their consumer threads on creation and expose a
`snd: Sender<FemtoLogRecord>` to the logger. The logger clones this sender when
created, ensuring log messages are dispatched without blocking. Dropping the
sender signals the consumer to finish once the queue is drained.

#### RotatingFileHandler

`FemtoRotatingFileHandler` mirrors Python's `RotatingFileHandler` and rotates
when the log file exceeds a configured `max_bytes`.

By default `max_bytes` and `backup_count` are `0`. A `max_bytes` of `0`
disables rotation entirely, and a `backup_count` of `0` retains no history.
These defaults are consistent across the Rust builder and Python API.

- The worker thread evaluates rotation without blocking producers by computing
  `current_file_len + next_record_bytes > max_bytes` using the formatted record
  length to avoid flush-induced drift.
- If `next_record_bytes` alone exceeds `max_bytes`, the worker triggers an
  immediate rollover whenever rotation is enabled. After the cascade it
  truncates the base file and writes the oversized record to the freshly
  truncated base file in full, mirroring CPython so no record is split across
  files.
- Rotation closes the active file handle before cascading existing backups from
  the highest index to the lowest (for example, ``<base>.3`` → ``<base>.4``, …,
  ``<base>`` → ``<base>.1``), then opens a fresh base file. Files beyond
  `backup_count` are deleted after the cascade.
- Filename indices start at `1` and increase sequentially up to
  `backup_count`; rollover prunes any files numbered above that cap.
- `max_bytes` and `backup_count` are surfaced through the Rust builder and
  Python API to keep configuration familiar.

## Thread Safety Considerations

- `FemtoLogRecord` and any user data it carries must implement `Send` so records
  can cross threads. Non‑`Send` values should be formatted into strings on the
  producer side.
- Handler state (stream, file, formatter) is encapsulated inside the consumer
  thread. Only the `Sender` is shared with producer threads, eliminating the
  need for additional locks.
- The default bounded capacity of 1024 records from
  [`dependency-analysis.md`](./dependency-analysis.md) prevents unbounded
  memory usage if a consumer stalls.
- No `unsafe` blocks are necessary; all concurrency primitives come from `std`
  or `crossbeam-channel`.

## Testing

Rust unit tests use the `rstest` crate, as shown in
[`rust-testing-with-rstest- fixtures.md`][rstest-fixtures]. Handlers should
expose minimal hooks (e.g. returning formatted strings in test mode) so tests
can verify output without relying on external I/O. Integration tests will
instantiate loggers and handlers together to ensure proper channel operation
and thread termination.

## Next Steps

1. Implement the `FemtoFormatter` trait and `DefaultFormatter` in Rust.
2. Port `StreamHandler` and `FileHandler` following the producer–consumer model.
3. Update the roadmap once these components have stable tests.
4. Expand with rotating and network handlers as described in the roadmap's later
   phases.

[cmhp-log]:
./concurrency-models-in-high-performance-logging.md#1-the-picologging-concurrency-model-a-hybrid-approach
