# Porting Formatters and Handlers to Rust

This document outlines a safe and thread‑aware design for moving formatting and
handler components from Python to Rust. It complements the
[roadmap](./roadmap.md) and expands on the design ideas described in [design
doc].

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
use cases can specify an overflow policy when constructing a handler. The
Python API exposes this via `OverflowPolicy` and
`FemtoFileHandler.with_capacity_flush_policy`. The policy may also be extended
to support options like back pressure, writing overflowed messages to a
separate file, or emitting metrics for monitoring purposes:

- **Drop** – current default; records are discarded when the queue is full.
- **Block** – the call blocks until space becomes available.
- **Timeout** – wait for a fixed duration before giving up and dropping the
  record.

Every handler provides a `flush()` method, so callers can force pending
messages to be written before shutdown.

```python
from femtologging import FemtoFileHandler, OverflowPolicy, PyHandlerConfig

# Drop-in replacement that blocks instead of discarding.
# Waiting ensures no log messages are lost if the queue becomes full.
config = PyHandlerConfig(
    capacity=4096,
    flush_interval=1,
    policy=OverflowPolicy.BLOCK.value,
    timeout_ms=None,
)
handler = FemtoFileHandler.with_capacity_flush_policy("app.log", config)
```

### StreamHandler

`FemtoStreamHandler` writes formatted records to `stdout` or `stderr`. The
consumer thread receives `FemtoLogRecord` values, moves the writer and
formatter into the worker thread, and writes directly without locking. This
mirrors the design in
[`concurrency-models-in-high-performance- logging.md`][cmhp-log]. The default
bounded queue size is 1024 records, but `FemtoStreamHandler::with_capacity`
lets callers configure a custom capacity when needed.

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

1. `config.rs` – all configuration structures, including
   `HandlerConfig`, `PyHandlerConfig` and `OverflowPolicy`.
2. `worker.rs` – the background writer thread and its helper types.
3. `mod.rs` – the public `FemtoFileHandler` API re‑exporting the config types.

```rust
use femtologging_rs::handlers::file::{FemtoFileHandler, HandlerConfig};
```

`FemtoFileHandler` exposes `flush()` and `close()` methods, so callers can
drain pending records and stop the background thread explicitly. Dropping the
handler still performs this cleanup if the methods aren't invoked.

By default, the file handler flushes the underlying file after every record to
maximize durability. To reduce syscall overhead in high-volume scenarios,
`FemtoFileHandler.with_capacity_flush()` accepts a `flush_interval` parameter
controlling how many records are written before the worker thread flushes.
Passing `0` disables periodic flushing and flushes only when the handler shuts
down.

The worker thread begins processing records as soon as the handler is created.
Production code therefore leaves the optional `start_barrier` field unset. Unit
tests may use this barrier to synchronise multiple workers and avoid race
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
 [design doc]: ./rust-multithreaded-logging-framework-for-python-design.md
