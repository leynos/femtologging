# Roadmap: Port picologging to Rust/PyO3

<!-- markdownlint-disable-next-line MD013 MD039 --> The design document in
[`socket handler design document`][socket-doc] outlines a phased approach for
building `femtologging`. The high‑level goal is to re‑implement picologging in
Rust with strong compile‑time safety and a multithreaded handler model. The
steps below summarize the actionable items from that design.

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
      - [x] Close-and-rename behaviour passes on Windows (no renaming of open
        files).
      - [x] Assert rotation happens on the worker thread and producers remain
        non-blocking under load.
- [x] Add `FemtoSocketHandler` with serialization (e.g. MessagePack or CBOR) and
  reconnection handling.
  - [x] Finalize the transport surface by supporting TCP and Unix domain socket
    addresses through the builder API and Python bindings, matching the
    configuration patterns described in
    [`configuration-design.md`](./configuration-design.md). Model the transport
    as an enum such as `Tcp { host, port, tls } | Unix { path }`, validate
    mutual exclusivity in the builder, split connect and write timeouts, and
    document IPv4 and IPv6 host support alongside TLS options.
  - [x] Implement a consumer-thread event loop that acquires sockets and
    serializes `FemtoLogRecord` values with `serde` (MessagePack or CBOR as
    identified in Section 3.4 of the [`socket handler design`][socket-doc].
    - [x] Frame each payload with a 4-byte big-endian length prefix, enforce a
      configurable maximum frame size (default 1 MiB), and handle partial
      writes or `EAGAIN` by buffering or dropping with metrics without blocking
      producers. Assert parity with Python's `logging.handlers.SocketHandler`
      framing rules.
  - [x] Add reconnection logic that respects exponential backoff and honours
    the multithreading and GIL-handling constraints captured in
    [`multithreading-in-pyo3.md`](./multithreading-in-pyo3.md), ensuring the
    GIL is not held across blocking network calls. Define parameters for
    `backoff_base`, `backoff_cap`, full jitter calculation, reset-after-success
    handling, and a maximum retry deadline, and require socket timeouts plus
    cooperative cancellation on shutdown. Ship concrete defaults of a
    `backoff_base` of 100 ms, a `backoff_cap` of 10 s, full jitter spanning
    `0..=current_interval`, reset-after-success triggered after 30 s of healthy
    writes, and a maximum retry deadline of 2 minutes before surfacing a
    handler error. Document where the builder methods and Python configuration
    knobs override each value.
  - [x] Provide error mapping that translates socket and serialization failures
    into Rust error enums and exported Python exceptions, following the
    guidance in the [`socket handler design document`][socket-doc].
  - [x] Extend the builder and Python configuration interfaces with
    `SocketHandlerBuilder`, including validation routines, documentation, and
    doctests kept dry per
    [`rust-doctest-dry-guide.md`](./rust-doctest-dry-guide.md).
  - [x] Write integration tests in Rust and Python using `rstest` fixtures (see
    [`rust-testing-with-rstest-fixtures.md`][rstest-doc]) to validate
    serialization framing, reconnection behaviour, IPv6/TLS error handling, and
    configuration round-tripping. Include parity tests against Python's
    `SocketHandler` covering frame handling, TLS handshake/verification
    failures, backoff jitter distribution, and `dictConfig`/`basicConfig`
    round-trips.
  - [x] When the above tasks are complete, mark `FemtoSocketHandler` as done in
    this roadmap and propagate the status to the configuration roadmap.

[socket-doc]: ./rust-multithreaded-logging-framework-for-python-design.md
[rstest-doc]: ./rust-testing-with-rstest-fixtures.md

- [x] Define the `FemtoFilter` trait and implement common filter
  types.[^1]
- [x] Support dynamic log level updates at runtime using atomic variables.
  - [x] Store each logger's threshold in an `AtomicU8` to enable lock‑free
    reads and writes across producer and consumer threads.
  - [x] Provide `FemtoLogger::set_level()` and expose
    `FemtoLogger.set_level()` in Python, so configuration APIs can adjust
    levels dynamically.
  - [x] Ensure log filtering consults the atomic level before formatting and
    dispatch, so dropped records never reach handlers.
  - [x] Cover runtime updates with Rust unit tests and Python integration
    tests (using `rstest` fixtures where appropriate), including invalid level
    rejection.
  - [x] Document runtime level semantics and the Python surface area in
    `rust-extension.md` and the configuration design notes.
- [ ] Implement the `log::Log` trait for compatibility with the `log` crate.
- [x] Expand test coverage and start benchmarking.

## Phase 3 – Advanced Features & Ecosystem Integration

- [ ] Implement `FemtoHTTPHandler` for sending logs over HTTP.
  - [ ] Resolve open design questions documented in the
    [`FemtoHTTPHandler Design`][http-design] section of the design document
    (serialization format, HTTP client library, retry semantics,
    `mapLogRecord` equivalent).
  - [ ] Implement HTTP transport configuration supporting URL, method
    (GET/POST), HTTPS, credentials, and custom headers through the builder
    API and Python bindings, following the preliminary architecture in
    [http-design].
  - [ ] Implement a consumer-thread event loop following the
    `FemtoSocketHandler` pattern, with HTTP request dispatch and payload
    preparation per the resolved `mapLogRecord` design.
  - [ ] Add retry logic reusing `BackoffPolicy` from `FemtoSocketHandler`,
    classifying HTTP status codes per the design document, and honouring the
    GIL-handling constraints in
    [`multithreading-in-pyo3.md`](./multithreading-in-pyo3.md).
  - [ ] Extend the builder and Python configuration interfaces with
    `HTTPHandlerBuilder`, including `dictConfig`/`basicConfig` integration
    and doctests kept dry per
    [`rust-doctest-dry-guide.md`](./rust-doctest-dry-guide.md).
  - [ ] Write integration tests using `rstest` fixtures (see
    [`rust-testing-with-rstest-fixtures.md`][rstest-doc]), including CPython
    `logging.HTTPHandler` parity tests.
  - [ ] Mark complete and propagate status to configuration roadmap.

[http-design]:
./rust-multithreaded-logging-framework-for-python-design.md#femtohttphandler-design

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
