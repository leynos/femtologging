# Roadmap: Port picologging to Rust/PyO3

<!-- markdownlint-disable-next-line MD013 MD039 --> The design document in
[`rust-multithreaded-logging-framework-for-python-design.md`](./rust-multithreaded-logging-framework-for-python-design.md)
 outlines a phased approach for building `femtologging`. The high‑level goal is
to re‑implement picologging in Rust with strong compile‑time safety and a
multithreaded handler model. The steps below summarize the actionable items
from that design.

## Initial Setup Tasks

- [x] Review picologging codebase and isolate core logging features
- [x] Evaluate dependencies and identify Rust equivalents
- [x] Design Rust crate layout and expose PyO3 bindings
- [x] Implement basic logger in Rust with matching Python API
- [x] Integrate Rust extension into Python packaging workflow
- [ ] Port formatting and handler components to Rust
- [ ] Add concurrency support and thread safety guarantees
- [ ] Benchmark against picologging and optimize hot paths
- [ ] Provide unit and integration tests for all features
- [ ] Set up continuous integration for Rust and Python tests
- [ ] Write migration guide for existing picologging users
- [ ] Publish femtologging package and update documentation

## Phase 1 – Core Functionality & Minimum Viable Product

- [ ] Define the `FemtoLogRecord` structure and implement core `FemtoLogger`
  logic, including efficient level checking and logging macros.
  - [x] Expand `FemtoLogRecord` with timestamp and source location. Thread info
    and structured key‑values are stored on each record.
  - [x] Add a `FemtoLevel` enum and per‑logger level checks.
  - [ ] Provide `debug!`, `info!`, `warn!`, and `error!` macros that capture
    source location.
  - [x] Route records to all configured handlers.
  - [x] Support attaching multiple handlers to a single logger.
  - [x] Allow a handler instance to be shared by multiple loggers safely.
- [x] Build a `Manager` registry, so `get_logger(name)` returns existing loggers
  and establishes parent relationships based on dotted names.
- [ ] Implement `propagate` behaviour so loggers inherit configuration from
  their parents up to the root logger.
- [x] Implement the `FemtoFormatter` trait with a default formatter.
- [x] Select and integrate an MPSC channel for producer‑consumer queues.
- [x] Create `FemtoStreamHandler` and `FemtoFileHandler`, each running in a
  dedicated consumer thread.
- [x] Provide a programmatic configuration API using the builder pattern.
- [ ] Add compile‑time log level filtering via Cargo features.
- [x] Ensure all components satisfy `Send`/`Sync` requirements.
- [x] Establish a basic test suite covering unit and integration tests.

## Phase 2 – Expanded Handlers & Core Features

- [x] Implement `femtologging.basicConfig()` translating to the builder API
  (see [configuration design](./configuration-design.md#basicconfig) and
  [example](../examples/basic_config.py)).
- [x] Implement `femtologging.dictConfig()` translating to the builder API.
- [ ] Implement `FemtoRotatingFileHandler` and `FemtoTimedRotatingFileHandler`
  with their respective rotation logic.
  - `FemtoRotatingFileHandler`:
    - [x] Expose `max_bytes` and `backup_count` options in Rust builders and
      Python wrappers.
    - [x] Check file size in the worker thread and trigger rotation without
      blocking producers.
      - [x] Define the predicate as UTF-8 byte measurement:
        `current_file_len + buffered_bytes + next_record_bytes > max_bytes` (do
        not flush solely to measure).
    - [x] Implement rotation algorithm that cascades file renames from highest
      to lowest index before opening a new file.
    - [x] Provide a filename strategy producing `<path>.<n>` sequences starting
      at `1` and capping at `backup_count`, pruning anything beyond that cap.
    - [x] Add builder and Python tests verifying size-based rollover.
      - [x] Cover boundaries: exactly `max_bytes`, one byte over, and an
        individual record larger than `max_bytes`.
      - [x] Verify records containing multi-byte UTF-8 characters trigger
        rotation based on byte length, not character count.
      - [x] Verify `backup_count == 0` truncates base file with no backups.
      - [x] Verify lowering `backup_count` prunes excess backups on the next
        rollover.
      - [x] Verify cascade renames run highest→lowest and never overwrite
        existing files.
      - [ ] Close-and-rename behaviour passes on Windows (no renaming of open
        files).
      - [ ] Assert rotation happens on the worker thread and producers remain
        non-blocking under load.
- [ ] Add `FemtoSocketHandler` with serialization (e.g. MessagePack or CBOR) and
  reconnection handling.
- [x] Define the `FemtoFilter` trait and implement common filter
  types.[^1]
- [ ] Support dynamic log level updates at runtime using atomic variables.
- [ ] Implement the `log::Log` trait for compatibility with the `log` crate.
- [ ] Expand test coverage and start benchmarking.

## Phase 3 – Advanced Features & Ecosystem Integration

- [ ] Implement `FemtoHTTPHandler` for sending logs over HTTP.
- [ ] Provide a `tracing_subscriber::Layer` so femtologging handlers can process
  `tracing` spans and events.
- [ ] Add file‑based configuration support (TOML or YAML via `serde`).
- [ ] Improve structured logging in macros to make context propagation easier.
- [ ] Investigate runtime reconfiguration of handlers and filters.
- [ ] Explore batching optimizations in consumer threads.

These phases will lead to a robust Rust implementation that matches the
performance goals of picologging while leveraging Rust's safety guarantees.
Development should start with Phase 1 to deliver a minimal, testable product
and iterate from there.

[^1]: Completed in PR [#198](https://github.com/leynos/femtologging/pull/198)
      on 6 September 2025.
