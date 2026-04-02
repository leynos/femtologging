# Architectural decision record (ADR) 004: batching optimizations in consumer threads

## Status

Proposed.

## Date

2026-03-24

## Context and Problem Statement

Every `femtologging` handler runs a dedicated consumer thread that receives
`FemtoLogRecord` instances from a bounded `crossbeam-channel` and processes
them one at a time. Each record triggers an independent I/O operation: a
`writeln!` plus `flush` for the stream handler, a formatted write (with
periodic flush) for the file handler, a full HTTP request-response round trip
for the HTTP handler, and a framed socket write plus `flush` for the socket
handler.

Under sustained high-volume logging, per-record I/O imposes measurable overhead:

- **System call frequency.** One `write` plus one `flush` per record means two
  kernel transitions per log line for stream and file handlers.
- **Network round trips.** The HTTP handler issues one HTTP request per record.
  Even with connection reuse (`ureq` agent pooling), TCP acknowledgement
  latency and TLS framing overhead accumulate.
- **Socket framing.** The socket handler serializes, length-prefixes, writes,
  and flushes each record individually, producing many small packets.
- **Lock contention.** Although each handler owns its writer exclusively,
  kernel-side locking on file descriptors and socket buffers is entered once
  per record.

The design document identifies this opportunity in §5.4[^1] and marks it as a
Phase 3 exploration item in §8.1[^2]. Roadmap item 2.3.3 formalizes the task.

This ADR analyses the batching strategies available, weighs their trade-offs,
and proposes a direction for implementation.

## Decision Drivers

- Reduce per-record I/O system call overhead for file and stream handlers.
- Amortize network round-trip costs for HTTP and socket handlers.
- Preserve the existing deterministic shutdown and drain semantics.
- Maintain tail-latency guarantees so that low-traffic loggers do not suffer
  unbounded buffering delays.
- Keep the batching mechanism opt-in or backward-compatible with current
  handler behaviour.
- Avoid introducing dependencies beyond `crossbeam-channel` for the core
  batching primitive.

## Requirements

- Reduce per-record I/O syscall overhead for file and stream handlers under
  sustained load.
- Amortize network round-trip and framing overhead for HTTP and socket
  handlers.
- Preserve deterministic shutdown semantics so dropping a sender still drains
  all queued records before worker exit.
- Preserve the existing flush acknowledgement contract for explicit flush
  operations.
- Maintain current tail-latency characteristics under light traffic by
  avoiding time-based dwell before the first record in a batch is processed.
- Keep batching opt-in or backward-compatible with existing handler
  configuration and overflow semantics.
- Avoid introducing new dependencies beyond `crossbeam-channel` for the core
  batching primitive.
- Keep the implementation local to handler worker loops so rollout remains
  incremental and testable.

## Options Considered

### Option A: drain-loop batching with `try_recv`

After receiving the first record via blocking `recv()`, the consumer
immediately drains additional records using a non-blocking `try_recv()` loop up
to a configurable `batch_capacity` ceiling. The collected batch is then
processed as a single I/O operation (one vectored write, one HTTP POST with a
JSON array body, or one concatenated socket frame sequence).

**Strengths:**

- Zero additional latency in the common case: if only one record is
  available, the batch contains one record and processing proceeds immediately.
- No timer thread or timeout mechanism required.
- Natural backpressure: batch sizes grow organically under load and shrink to
  one under light traffic.
- Straightforward to implement using `crossbeam-channel`'s existing
  `try_recv()`.

**Weaknesses:**

- Under light load, batches are always of size one — no amortization benefit.
- No configurable maximum dwell time; records are never held waiting for peers.

### Option B: time-bounded batching with deadline

The consumer collects records into a local buffer, flushing the batch either
when the buffer reaches `batch_capacity` or when a `batch_timeout` duration
elapses since the first record in the current batch was received, whichever
comes first.

**Strengths:**

- Provides a configurable upper bound on individual record latency
  (`batch_timeout`).
- Under moderate load, batches fill more evenly, yielding consistent I/O
  amortization.

**Weaknesses:**

- Requires a timer mechanism. `crossbeam-channel` `select!` supports
  `recv_timeout` and `select_timeout`, but interleaving timeout branches with
  shutdown signals increases control-flow complexity.
- Under light load, every record incurs up to `batch_timeout` latency before
  emission, which is undesirable for interactive or debug logging.
- Adds two configuration knobs (`batch_capacity`, `batch_timeout`) per
  handler, increasing the configuration surface.

### Option C: vectored I/O with `writev` (file and stream handlers only)

Rather than batching at the application level, use `writev(2)` scatter-gather
I/O to submit multiple pre-formatted byte slices in a single system call.
Records are formatted eagerly and their byte representations are accumulated
into an `IoSlice` array, which is flushed via a single `write_vectored` call.

**Strengths:**

- Kernel-level batching: one system call writes multiple records.
- No application-level buffering delay; records are formatted immediately.
- Rust's `std::io::Write::write_vectored` provides a safe abstraction.

**Weaknesses:**

- Only applicable to file and stream handlers; HTTP and socket handlers
  require application-level batching regardless.
- `write_vectored` may fall back to sequential writes on some platforms if the
  underlying file descriptor does not support scatter-gather.
- Does not reduce formatting overhead, only write system calls.

### Option D: channel swap to `flume` with `drain` / `try_iter`

Replace `crossbeam-channel` with `flume`, which exposes `try_iter()` and
`drain()` iterators that yield all immediately available messages.

**Strengths:**

- `flume::Receiver::try_iter()` provides idiomatic batch collection.
- `flume` offers comparable performance to `crossbeam-channel` in benchmarks
  and adds `async` send/recv for potential future use.

**Weaknesses:**

- Introduces a dependency change across the entire codebase.
- `crossbeam-channel` already supports `try_recv()` in a loop, which achieves
  the same outcome without a crate swap.
- `crossbeam-channel`'s `select!` macro (used in the logger worker thread) has
  no direct `flume` equivalent; migration requires restructuring the shutdown
  signalling mechanism.
- Risk of subtle behavioural differences during migration.

### Comparison

| Criterion                      | A: drain-loop | B: time-bounded | C: `writev` | D: `flume` swap |
| ------------------------------ | ------------- | --------------- | ----------- | --------------- |
| I/O amortization under load    | High          | High            | High        | High            |
| Latency under light load       | None added    | Up to timeout   | None added  | None added      |
| Implementation complexity      | Low           | Medium          | Low         | High            |
| Handler coverage               | All           | All             | File/stream | All             |
| New dependencies               | None          | None            | None        | `flume`         |
| Configuration surface growth   | +1 knob       | +2 knobs        | None        | +1 knob         |
| Shutdown/drain semantic change | Minimal       | Moderate        | None        | Moderate        |

_Table 1: Trade-offs between batching strategies._

## Decision Outcome / Proposed Direction

Adopt **Option A (drain-loop batching)** as the primary strategy. For
`FemtoFileHandler` and `FemtoStreamHandler`, use a **single contiguous buffer
plus `write_all`** as the canonical Phase 1 batch write strategy. True
scatter-gather I/O remains a future optimization once `write_all_vectored`
stabilizes.

**Rationale:**

Option A provides the best trade-off between implementation simplicity,
performance under load, and zero added latency under light traffic. It requires
no new dependencies, no timer threads, and no changes to the shutdown
signalling architecture. Under sustained load — the scenario where batching
matters most — `try_recv()` naturally collects large batches. Under light
traffic, it degrades gracefully to the current one-at-a-time behaviour.

For Phase 1, the accepted file and stream write path is:

- format each record into a newline-terminated byte buffer;
- concatenate the batch into one contiguous `Vec<u8>`;
- call `write_all` once for the concatenated payload;
- flush once at the batch boundary, or immediately before acknowledging an
  explicit `Flush`.

This keeps the implementation on stable Rust while still collapsing multiple
record writes into one batch write. If true vectored writes become practical in
stable Rust, the handlers can switch internally without changing the external
batching contract. Until then, there is no secondary `write_vectored` path in
Phase 1, so the fallback rule is simple: `FemtoFileHandler` and
`FemtoStreamHandler` always use the contiguous-buffer `write_all` path.

Options B, C, and D are not selected at this time. Option B's added latency
under light load is unacceptable for debug and interactive logging without
significant configuration effort. Option C still depends on unstable
`write_all_vectored` for a robust stable-Rust implementation, and Option D
introduces migration risk for marginal ergonomic gain over Option A.

## Goals and Non-goals

### Goals

- Introduce a `BatchConfig` structure controlling maximum batch size.
- Implement drain-loop batching in all four handler worker threads (stream,
  file, HTTP, socket).
- Use a contiguous batch buffer plus `write_all` for file and stream handlers
  during Phase 1.
- Preserve existing shutdown drain semantics: all queued records are processed
  before the worker thread exits.
- Preserve the existing flush-acknowledge protocol used by `handle_flush`.
- Provide a default `batch_capacity` that is effective under load without
  requiring user configuration.
- Add Criterion benchmarks measuring throughput with and without batching.

### Non-goals

- Replacing `crossbeam-channel` with another channel crate.
- Introducing time-bounded batching or dwell timeouts in this iteration.
- Changing the channel capacity or backpressure/overflow policy semantics.
- Batching across multiple handlers (each handler batches independently).

## Code Sketches

The following sketches illustrate the proposed approach for the chosen
direction. They are simplified for clarity; production code will include full
error handling, doc comments, and tests.

### Batch collection primitive

A shared helper collects records from the channel after the first blocking
receive. All handler workers can reuse this function.

```rust,no_run
use crossbeam_channel::{Receiver, RecvError};

/// Collects a batch of items from `rx`.
///
/// Blocks until the first item arrives, then drains up to
/// `max_batch - 1` additional items that are immediately available.
/// The internal `Vec` is preallocated to `max_batch` because the
/// caller controls the ceiling; the proposed `BatchConfig`
/// configuration type introduced later in this document validates
/// that ceiling before batched workers start.
///
/// # Returns
///
/// `Ok(batch)` with at least one item, or `Err(RecvError)` when
/// the channel is disconnected and empty.
fn recv_batch<T>(rx: &Receiver<T>, max_batch: usize) -> Result<Vec<T>, RecvError> {
    let first = rx.recv()?;
    let mut batch = Vec::with_capacity(max_batch);
    batch.push(first);
    while batch.len() < max_batch {
        match rx.try_recv() {
            Ok(item) => batch.push(item),
            Err(_) => break,
        }
    }
    Ok(batch)
}
```

### File handler worker loop (batched)

`FileCommand::Flush` should carry a per-command acknowledgement sender so file
and stream handlers follow the same flush contract before drain-loop batching
lands. Like `StreamCommand`, the enum does not carry a `Shutdown` variant
because orderly shutdown is signalled by dropping the command sender, which
causes `recv_batch` to return `Err(RecvError)` and the loop to exit.

```rust,no_run
/// Commands sent to the file handler worker thread.
pub enum FileCommand {
    /// Write a log record to the underlying writer.
    Record(Box<FemtoLogRecord>),
    /// Flush the writer and send an ack on the provided channel.
    Flush(Sender<io::Result<()>>),
}
```

The file handler collects a batch, formats each record, concatenates the
formatted lines into a single stable-Rust buffer, writes that buffer with
`write_all`, and flushes once per batch. `state.write_batch` performs only the
contiguous-buffer `write_all`; it does not flush. A single `state.flush_once`
call follows once all records in the batch have been written.

If a `Flush` command appears mid-batch, all accumulated record lines must be
written **and** flushed **before** the acknowledgement is sent. In the
implemented contract, `FileCommand::Flush` acknowledges only the result of the
writer flush itself, so `FemtoFileHandler::wait_for_flush_completion` treats
`Ok(Ok(()))` as success. Earlier `handle_record` failures are still logged at
write time and are not folded into the later flush acknowledgement. In the
sketch below, `state.write_batch` writes the pending lines, `state.flush_once`
flushes the writer, and only then is the ack sent via the per-command
`Sender<io::Result<()>>`. Remaining records in the batch continue into a fresh
buffer.

```rust,no_run
use std::io::{self, IoSlice, Write};

fn run_batched_file_worker<W, F, R>(
    rx: Receiver<FileCommand>,
    mut state: WorkerState<W, R>,
    formatter: F,
    done_tx: Sender<()>,
    batch_capacity: usize,
) where
    W: Write + Seek,
    F: FemtoFormatter,
    R: RotationStrategy<W>,
{
    loop {
        match recv_batch(&rx, batch_capacity) {
            Ok(commands) => {
                let mut formatted: Vec<Vec<u8>> = Vec::new();
                for cmd in commands {
                    match cmd {
                        FileCommand::Record(record) => {
                            let msg = formatter.format(&record);
                            let mut line = msg.into_bytes();
                            line.push(b'\n');
                            formatted.push(line);
                        }
                        FileCommand::Flush(ack) => {
                            // Write all pending lines, flush, then ack.
                            // The ack must not be sent until every
                            // preceding record has reached the writer.
                            if !formatted.is_empty() {
                                state.write_batch(&formatted);
                                formatted.clear();
                            }
                            let _ = ack.send(state.flush_once());
                        }
                    }
                }
                if !formatted.is_empty() {
                    state.write_batch(&formatted);
                    state.flush_once();
                }
            }
            Err(_) => break, // sender dropped → orderly shutdown
        }
    }
    state.final_flush();
    let _ = done_tx.send(());
}
```

### Stream handler worker loop (batched with contiguous-buffer writes)

The existing `StreamCommand` enum should carry a per-command
`Sender<io::Result<()>>` in the `Flush` variant so the caller can wait for the
specific flush result, matching `FileCommand::Flush`. Like `FileCommand`, there
is no `Shutdown` variant: orderly shutdown is signalled by dropping the command
sender, causing `recv_batch` to return `Err(RecvError)`.

```rust,no_run
/// Commands sent to the stream handler worker thread.
enum StreamCommand {
    Record(FemtoLogRecord),
    /// Flush the writer and send an ack on the provided channel.
    Flush(Sender<io::Result<()>>),
}
```

Each batch results in exactly one contiguous-buffer `write_all` call (to submit
the formatted lines) followed by exactly one `flush` call (to ensure the data
reaches the underlying stream). If a `Flush` command appears mid-batch, the
accumulated lines are written and flushed before acknowledging; remaining
records in the same batch continue into a fresh buffer. The helper
`write_batch_buffered` performs only the buffered write, so the caller controls
when and how often `flush` is invoked. As with the file handler,
`StreamCommand::Flush` acknowledges only the direct `writer.flush()` result;
earlier write failures remain warning-only events rather than being folded into
the later flush ack.

The current `FemtoStreamHandler` worker already drains queued commands before
shutdown because `for cmd in rx` only terminates once the sender is dropped
_and_ the channel is empty. The batched drain-loop variant must preserve that
behaviour by fully processing the backlog collected after each blocking receive
before calling `final_flush`.

```rust,no_run
use std::io;

fn run_batched_stream_worker<W, F>(
    rx: Receiver<StreamCommand>,
    mut writer: W,
    formatter: F,
    done_tx: Sender<()>,
    batch_capacity: usize,
) where
    W: Write,
    F: FemtoFormatter,
{
    loop {
        match recv_batch(&rx, batch_capacity) {
            Ok(commands) => {
                let mut formatted: Vec<Vec<u8>> = Vec::new();
                for cmd in commands {
                    match cmd {
                        StreamCommand::Record(record) => {
                            let msg = formatter.format(&record);
                            let mut line = msg.into_bytes();
                            line.push(b'\n');
                            formatted.push(line);
                        }
                        StreamCommand::Flush(ack) => {
                            // Drain accumulated lines, then flush once.
                            let write_result =
                                write_batch_buffered(&mut writer, &formatted);
                            if write_result.is_ok() {
                                formatted.clear();
                            }
                            let flush_result = write_result.and_then(|()| writer.flush());
                            let _ = ack.send(flush_result);
                        }
                    }
                }
                // Write remaining lines and flush once for the batch.
                if !formatted.is_empty() {
                    if let Err(err) = write_batch_buffered(&mut writer, &formatted) {
                        log::warn!("FemtoStreamHandler batch write error: {err}");
                    } else {
                        formatted.clear();
                        let _ = writer.flush();
                    }
                }
            }
            Err(_) => break,
        }
    }
    let _ = writer.flush();
    let _ = done_tx.send(());
}

/// Writes all pre-formatted lines to `writer`, handling short writes.
///
/// Concatenates the lines into a single contiguous buffer and calls
/// `write_all`, which loops internally until every byte is written.
/// A single-buffer approach is used because `write_all_vectored`
/// remains nightly-only (tracking issue [#70436][wa]).  By contrast,
/// `IoSlice::advance_slices` was stabilized in Rust 1.81.0, so the
/// remaining blocker for true scatter-gather I/O is the nightly
/// `write_all_vectored` API. If `write_all_vectored` is stabilized in a
/// future Rust release the implementation can switch to true
/// scatter-gather I/O without changing the function signature.
///
/// This function does **not** flush — the caller is responsible for
/// calling `flush` at the appropriate batch boundary.
///
/// [wa]: https://github.com/rust-lang/rust/issues/70436
fn write_batch_buffered<W: Write>(
    writer: &mut W,
    lines: &[Vec<u8>],
) -> io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    let total: usize = lines.iter().map(|l| l.len()).sum();
    let mut buf = Vec::with_capacity(total);
    for line in lines {
        buf.extend_from_slice(line);
    }
    writer.write_all(&buf)
}
```

The same contiguous-buffer rule applies to `FemtoFileHandler` in Phase 1. If a
future stable Rust release makes `write_all_vectored` available, both file and
stream handlers can switch to a true scatter-gather implementation behind the
same batch-processing contract. Until then, `WouldBlock` or partial-write
fallback logic is unnecessary because the accepted implementation always uses a
single `write_all` call over the concatenated buffer.

### HTTP handler worker loop (batched JSON array)

The HTTP handler collects a batch of records, serializes them into a single
JSON array payload, and sends one HTTP request per batch.

```rust,no_run
fn handle_record_batch(&mut self, records: Vec<FemtoLogRecord>) {
    let payloads: Vec<String> = records
        .iter()
        .filter_map(|r| self.serialize_record(r).ok())
        .collect();
    if payloads.is_empty() {
        return;
    }
    let batch_payload = match self.config.format {
        SerializationFormat::Json => {
            format!("[{}]", payloads.join(","))
        }
        SerializationFormat::UrlEncoded => {
            // URL-encoded batching: send records separated by newlines.
            payloads.join("\n")
        }
    };
    self.send_request(&batch_payload);
}
```

### Socket handler worker loop (batched frame writes)

The socket handler prepares frames for each record in the batch and writes them
in a single `write_all` call, flushing once.

```rust,no_run
fn handle_record_batch(&mut self, records: Vec<FemtoLogRecord>) {
    let mut combined_frames: Vec<u8> = Vec::new();
    for record in &records {
        match prepare_frame(record, &self.config) {
            Ok(frame) => combined_frames.extend_from_slice(&frame),
            Err(err) => {
                warn!("FemtoSocketHandler serialization error: {err}");
            }
        }
    }
    if combined_frames.is_empty() {
        return;
    }
    let now = Instant::now();
    self.send_combined_frames(&combined_frames, now);
}
```

### Logger worker thread (batched dispatch)

The logger worker thread uses `select!` with the shutdown channel. Batching
integrates by draining additional records after the first arrives.

```rust,no_run
fn worker_thread_loop(rx: Receiver<QueuedRecord>, shutdown_rx: Receiver<()>) {
    loop {
        if Self::should_shutdown_now(&shutdown_rx) {
            Self::shutdown_and_drain(&rx);
            break;
        }
        select! {
            recv(shutdown_rx) -> _ => {
                Self::shutdown_and_drain(&rx);
                break;
            },
            recv(rx) -> rec => match rec {
                Ok(first) => {
                    // Drain additional records that are immediately
                    // available, up to a batch ceiling.
                    let mut batch = Vec::with_capacity(64);
                    batch.push(first);
                    while batch.len() < 64 {
                        match rx.try_recv() {
                            Ok(item) => batch.push(item),
                            Err(_) => break,
                        }
                    }
                    for job in batch {
                        Self::handle_log_record(job);
                    }
                }
                Err(_) => break,
            },
        }
    }
}
```

### `BatchConfig` structure

`BatchConfig` uses an eager checked constructor in the current implementation.
This keeps zero-capacity errors close to the API boundary and avoids carrying
an invalid batch size deeper into the worker setup path.

```rust,no_run
/// Controls batching behaviour for handler consumer threads.
///
/// # Parameters
///
/// - `capacity`: Maximum number of records collected per batch.
///   Defaults to `64`. Setting this to `1` disables batching.
pub struct BatchConfig {
    capacity: usize,
}

impl BatchConfig {
    /// The default batch capacity.
    const DEFAULT_CAPACITY: usize = 64;

    /// Creates a new `BatchConfig` with the given capacity.
    ///
    /// # Errors
    ///
    /// Returns an error if `capacity` is zero.
    pub fn new(capacity: usize) -> Result<Self, BatchConfigError> {
        if capacity == 0 {
            return Err(BatchConfigError::ZeroCapacity);
        }
        Ok(Self { capacity })
    }
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            capacity: Self::DEFAULT_CAPACITY,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BatchConfigError {
    #[error("batch capacity must be greater than zero")]
    ZeroCapacity,
}
```

### Validation strategy

`FileHandlerBuilder::build_inner` and `StreamHandlerBuilder::build_inner`
should consume a checked `BatchConfig`, rather than staging an invalid batch
size and validating it later. In the current implementation, callers use
`BatchConfig::new(...)` at the point where a custom batch capacity is created,
and the worker configuration keeps `BatchConfig::default()` for the built-in
non-zero default. There is no separate `BatchConfig::validate()` phase.

## Migration Plan

### 1. Core batch collection and file/stream handler batching

- [ ] 1.1 Define the core batching primitives.
- [ ] 1.1.1 Introduce the `recv_batch` helper and `BatchConfig` type.
- [ ] 1.1.2 Default `batch_capacity` to 64 and expose it through the existing
      builder APIs.
- [ ] 1.2 Standardize the flush acknowledgement contract before enabling
      drain-loop batching.
- [ ] 1.2.1 Make `FileCommand::Flush` and `StreamCommand::Flush` both carry
      per-command `Sender<io::Result<()>>` values.
- [ ] 1.2.2 Thread the per-command flush sender through the handler
      construction and spawn paths used by `FemtoFileHandler`,
      `FemtoStreamHandler`, and their builder APIs.
- [ ] 1.3 Batch file and stream worker writes on stable Rust.
- [ ] 1.3.1 Modify `FemtoFileHandler` worker loops to use drain-loop batching
      with a contiguous batch buffer plus `write_all`.
- [ ] 1.3.2 Modify `FemtoStreamHandler` worker loops to use drain-loop
      batching with a contiguous batch buffer plus `write_all`.
- [ ] 1.4 Add Criterion benchmarks comparing single-record and batched
      throughput for file and stream handlers.

### 2. Network handler batching

- [ ] 2.1 Batch HTTP handler writes.
- [ ] 2.1.1 Modify `FemtoHTTPHandler` to serialize batch payloads as JSON
      arrays and send one request per batch.
- [ ] 2.2 Batch socket handler writes.
- [ ] 2.2.1 Modify `FemtoSocketHandler` to concatenate frames and write
      combined buffers.
- [ ] 2.3 Add Criterion benchmarks for HTTP and socket handler batched
      throughput.
- [ ] 2.4 Update the user guide with batching configuration guidance.

### 3. Logger dispatch batching and hardening

- [ ] 3.1 Batch logger dispatch.
- [ ] 3.1.1 Modify the logger worker thread to drain-loop batch records before
      dispatching to handlers.
- [ ] 3.2 Preserve drain and shutdown correctness.
- [ ] 3.2.1 Ensure shutdown drain semantics process all remaining records.
- [ ] 3.2.2 Add integration tests verifying record ordering, completeness
      under load, and graceful shutdown with batched workers.
- [ ] 3.3 Finalize the documentation and roadmap.
- [ ] 3.3.1 Update design document §5.4 and §8.1 to reflect the implemented
      approach.
- [ ] 3.3.2 Mark roadmap item 2.3.3 as complete.

## Known Risks and Limitations

- **Contiguous-buffer allocation cost.** File and stream batching trades extra
  copying into one `Vec<u8>` for fewer write calls. Large batches increase
  temporary allocation pressure even though the approach stays on stable Rust.
- **HTTP receiver compatibility.** Batched JSON array payloads require the
  receiving HTTP endpoint to accept arrays. Endpoints expecting single-object
  payloads will reject batched requests. This must be documented and the
  feature should be opt-in for HTTP handlers.
- **Record ordering.** Drain-loop batching preserves insertion order within a
  single handler's channel. Cross-handler ordering is already unspecified and
  remains so.
- **Memory pressure.** Collecting up to `batch_capacity` records into a local
  `Vec` temporarily doubles memory usage for those records (channel buffer plus
  local batch). With the default capacity of 64 and typical record sizes, this
  overhead is negligible.

## Outstanding Decisions

- Whether HTTP batch mode should be opt-in (requiring explicit configuration)
  or opt-out (enabled by default with a way to disable).
- Whether `BatchConfig` should be exposed through `dictConfig` and
  `fileConfig`, or limited to the programmatic builder API in the initial
  release.
- The exact default `batch_capacity` value; 64 is proposed but should be
  validated by benchmarking.
- Whether `FlushTracker` interval semantics should count individual records or
  batches after the change.

## Architectural Rationale

Drain-loop batching preserves femtologging's core architectural invariants:
dedicated consumer threads, bounded channels with backpressure, and
deterministic shutdown draining. It introduces no new concurrency primitives,
no timer threads, and no cross-handler coordination. The optimization is
localized entirely within each handler's worker loop, keeping the change
surface small and testable.

The contiguous-buffer write path for file and stream handlers keeps the Phase 1
implementation on stable Rust and avoids introducing platform-specific
scatter-gather branches before the standard library grows a stable
`write_all_vectored`. Together with drain-loop batching, this addresses the
primary I/O overhead identified in design §5.4[^1] without compromising the
latency characteristics that make femtologging suitable for interactive and
debug logging under light traffic.

[^1]: <./rust-multithreaded-logging-framework-for-python-design.md#54-potential-for-batching-log-messages-in-consumer-threads>
[^2]: <./rust-multithreaded-logging-framework-for-python-design.md#81-suggested-implementation-roadmap>
