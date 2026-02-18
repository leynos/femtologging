# Roadmap: Port picologging to Rust/PyO3

This roadmap consolidates the previous implementation and configuration
roadmaps into one execution plan. Tasks are grouped by strategic phase,
workstream, and measurable execution units, with citations to design sources
where applicable.

## 1. Foundation and core runtime

### 1.1. Baseline architecture and packaging

- [x] 1.1.1. Review picologging codebase and isolate core logging features. See
  [design §1](./rust-multithreaded-logging-framework-for-python-design.md#1-introduction).
- [x] 1.1.2. Evaluate dependencies and identify Rust equivalents. See
  [dependency analysis](./dependency-analysis.md) and
  [design §5](./rust-multithreaded-logging-framework-for-python-design.md#5-optimizing-for-performance).
- [x] 1.1.3. Design Rust crate layout and expose PyO3 bindings. See
  [design §3.1](./rust-multithreaded-logging-framework-for-python-design.md#31-key-components)
   and [rust extension](./rust-extension.md).
- [x] 1.1.4. Implement a basic logger in Rust with a matching Python API. See
  [design §3.1](./rust-multithreaded-logging-framework-for-python-design.md#31-key-components)
   and
  [design §6.1](./rust-multithreaded-logging-framework-for-python-design.md#61-balancing-api-familiarity-with-rust-idioms).
- [x] 1.1.5. Integrate the Rust extension into the Python packaging workflow.
  See [dev workflow](./dev-workflow.md#commands).

### 1.2. Logger model and thread-safe dispatch

- [ ] 1.2.1. Finalize `FemtoLogRecord` and core `FemtoLogger` behaviour. See
  [design §3.3](./rust-multithreaded-logging-framework-for-python-design.md#33-femtologrecord-structure)
   and
  [design §4.3](./rust-multithreaded-logging-framework-for-python-design.md#43-macros-for-ergonomic-and-safe-logging).
  - [x] Expand `FemtoLogRecord` with timestamp, source location, thread info,
    and structured key-values.
  - [x] Add `FemtoLevel` enum and per-logger level checks.
  - [ ] Provide `debug!`, `info!`, `warn!`, and `error!` macros that capture
    source location.
  - [x] Route records to all configured handlers.
  - [x] Support attaching multiple handlers to a single logger.
  - [x] Allow one handler instance to be shared by multiple loggers safely.
- [x] 1.2.2. Build a `Manager` registry so `get_logger(name)` reuses existing
  loggers and establishes dotted-name parent relationships. See
  [design §3.1](./rust-multithreaded-logging-framework-for-python-design.md#31-key-components)
   and
  [configuration design §6](./configuration-design.md#6-logger-propagation).
- [x] 1.2.3. Implement `propagate` behaviour from child loggers to the root
  logger. See
  [configuration design §6.1](./configuration-design.md#61-propagation-semantics).
- [x] 1.2.4. Select and integrate a multi-producer, single-consumer (MPSC)
  channel for producer-consumer queues. See
  [design §5.3](./rust-multithreaded-logging-framework-for-python-design.md#53-impact-of-mpsc-channel-choice-on-throughput-and-latency).
- [x] 1.2.5. Ensure all threaded components satisfy `Send`/`Sync` requirements.
  See
  [design §4.2](./rust-multithreaded-logging-framework-for-python-design.md#42-send-and-sync-traits-for-thread-safety).

### 1.3. Configuration builders and Python parity

- [x] 1.3.1. Implement `ConfigBuilder`, `LoggerConfigBuilder`, and
  `FormatterBuilder` in Rust. See
  [configuration design §1.1](./configuration-design.md#11-rust-builder-api-design).
- [x] 1.3.2. Implement `FileHandlerBuilder` and `StreamHandlerBuilder` in Rust,
  including `HandlerBuilderTrait` build semantics. See
  [configuration design §1.3](./configuration-design.md#13-implemented-handler-builders).
- [x] 1.3.3. Enable multiple handler IDs per logger and `Arc`-shared handler
  instances. See
  [configuration design §1.1](./configuration-design.md#11-rust-builder-api-design)
   and
  [configuration design §6](./configuration-design.md#6-logger-propagation).
- [x] 1.3.4. Expose parity Python builders (`ConfigBuilder`,
  `LoggerConfigBuilder`, `FormatterBuilder`, `FileHandlerBuilder`, and
  `StreamHandlerBuilder`) via PyO3. See
  [configuration design §1.2](./configuration-design.md#12-python-builder-api-design-congruent-with-rust-and-python-schemas).
- [x] 1.3.5. Establish a baseline Rust and Python test suite for builder flows,
  including integration coverage. See
  [configuration design §7](./configuration-design.md#7-testing-and-benchmarking-coverage)
   and [rstest guide](./rust-testing-with-rstest-fixtures.md).

## 2. Handler and transport expansion

### 2.1. Formatter and handler port completion

- [x] 2.1.1. Port formatter and core handler components to Rust
      (`FemtoFormatter`,
  `FemtoStreamHandler`, and `FemtoFileHandler`). See
  [formatters and handlers port](./formatters-and-handlers-rust-port.md) and
  [design §3.4](./rust-multithreaded-logging-framework-for-python-design.md#34-handler-implementation-strategy).
- [ ] 2.1.2. Complete rotating-handler coverage by adding
  `FemtoTimedRotatingFileHandler`. See
  [design §3.4](./rust-multithreaded-logging-framework-for-python-design.md#34-handler-implementation-strategy)
   and
  [design §6.3.1](./rust-multithreaded-logging-framework-for-python-design.md#631-rotating-file-handler-configuration-decisions).
- [x] 2.1.3. Implement `FemtoRotatingFileHandler` with non-blocking,
  worker-thread rotation semantics. See
  [design §6.3.1](./rust-multithreaded-logging-framework-for-python-design.md#631-rotating-file-handler-configuration-decisions)
   and
  [configuration design §1.3](./configuration-design.md#13-implemented-handler-builders).
  - [x] Expose `max_bytes` and `backup_count` in Rust builders and Python
    wrappers.
  - [x] Trigger rotation from the worker thread using UTF-8 byte accounting.
  - [x] Implement highest-to-lowest cascade renames with backup pruning.
  - [x] Add boundary tests for exact limit, overflow by one byte, and oversized
    single records.
  - [x] Verify multi-byte UTF-8 rotation checks are byte-based.
  - [x] Verify `backup_count == 0` truncates the base file without backups.
  - [x] Verify lowering `backup_count` prunes excess backups on next rollover.
  - [x] Verify Windows-safe close-and-rename behaviour.
  - [x] Assert producer non-blocking behaviour under load.

### 2.2. Network handlers

- [x] 2.2.1. Deliver `FemtoSocketHandler` with framed serialization and robust
  reconnection behaviour. See
  [socket design update](./rust-multithreaded-logging-framework-for-python-design.md#femtosockethandler-implementation-update)
   and [multithreading in PyO3](./multithreading-in-pyo3.md).
  - [x] Support TCP and Unix domain socket transports with builder and Python
    configuration parity.
  - [x] Serialize records with length-prefixed MessagePack/CBOR framing and
    enforce configurable frame limits.
  - [x] Implement backoff with jitter, retry deadline controls, and cancellation
    on shutdown.
  - [x] Map socket and serialization failures to Rust error enums and exported
    Python exceptions.
  - [x] Add `SocketHandlerBuilder` with validation and doctest coverage.
  - [x] Add Rust and Python integration tests, including CPython
    `SocketHandler` parity checks.
- [x] 2.2.2. Deliver `FemtoHTTPHandler` and `HTTPHandlerBuilder` with retry and
  parity-driven integration testing. See
  [HTTP design](./rust-multithreaded-logging-framework-for-python-design.md#femtohttphandler-design)
   and [rstest guide](./rust-testing-with-rstest-fixtures.md).

### 2.3. Concurrency controls and lifecycle

- [x] 2.3.1. Implement back-pressure controls via bounded queues and overflow
  policies. See
  [formatters and handlers port](./formatters-and-handlers-rust-port.md#femtohandler-trait-and-implementations)
   and
  [design §5.1](./rust-multithreaded-logging-framework-for-python-design.md#51-minimizing-overhead-at-the-log-call-site-the-hot-path).
- [x] 2.3.2. Implement graceful shutdown semantics for logger and handler worker
  threads. See
  [configuration design §3](./configuration-design.md#3-runtime-reconfiguration)
   and
  [logging sequence diagrams](./logging-sequence-diagrams.md#5-shutdown--orderly-shutdown-at-process-exit).
- [ ] 2.3.3. Explore batching optimizations in consumer threads. See
  [design §5.4](./rust-multithreaded-logging-framework-for-python-design.md#54-potential-for-batching-log-messages-in-consumer-threads)
   and
  [design §8.1](./rust-multithreaded-logging-framework-for-python-design.md#81-suggested-implementation-roadmap).

## 3. Configuration, compatibility, and ecosystem integration

### 3.1. Backwards-compatible configuration APIs

- [x] 3.1.1. Implement `femtologging.basicConfig()` as a translation to the
  builder API. See
  [configuration design §2.1](./configuration-design.md#21-basicconfig).
- [x] 3.1.2. Implement `femtologging.dictConfig()` with schema parsing,
  constructor argument handling, ordered section processing, and validation for
  unsupported features. See
  [configuration design §2.2](./configuration-design.md#22-dictconfig).
- [x] 3.1.3. Implement `femtologging.fileConfig()` by parsing INI content in
  Rust, converting to a `dictConfig`-compatible structure in Python, and
  delegating application through `dictConfig()`. See
  [configuration design §2.3](./configuration-design.md#23-fileconfig).
- [ ] 3.1.4. Add file-based configuration support for TOML or YAML via `serde`
  as a follow-on configuration format. See
  [design §7.3](./rust-multithreaded-logging-framework-for-python-design.md#73-integration-with-application-configuration).

### 3.2. Runtime controls and structured filtering

- [x] 3.2.1. Define `FemtoFilter` and common filter implementations
  (`LevelFilter`, `NameFilter`) with builder integration. See
  [configuration design §1.1.1](./configuration-design.md#111-filters).
- [x] 3.2.2. Support dynamic log-level updates with atomic storage and expose
  `set_level()` in Python. See
  [configuration design §3](./configuration-design.md#3-runtime-reconfiguration).
- [ ] 3.2.3. Implement compile-time log-level filtering via Cargo features. See
  [design §5.1](./rust-multithreaded-logging-framework-for-python-design.md#51-minimizing-overhead-at-the-log-call-site-the-hot-path)
   and
  [design §8.1](./rust-multithreaded-logging-framework-for-python-design.md#81-suggested-implementation-roadmap).
- [ ] 3.2.4. Extend runtime reconfiguration to support richer handler and filter
  mutation workflows beyond level changes. See
  [design §7.2](./rust-multithreaded-logging-framework-for-python-design.md#72-dynamic-reconfiguration)
   and
  [configuration design §3](./configuration-design.md#3-runtime-reconfiguration).

### 3.3. Rust ecosystem integration

- [x] 3.3.1. Implement `log::Log` compatibility with level mapping,
  target-to-logger routing, flush bridging, feature gating, and Rust/Python
  integration tests. See
  [design §6.4](./rust-multithreaded-logging-framework-for-python-design.md#64-interoperability-with-the-rust-logging-ecosystem)
   and
  [design §6.4.1](./rust-multithreaded-logging-framework-for-python-design.md#641-implementation-strategy-for-loglog).
- [x] 3.3.2. Provide migration notes for `log-compat`, including when and how to
  call `setup_rust_logging()` during start-up. See
  [rust extension](./rust-extension.md#rust-log-crate-bridge).
- [ ] 3.3.3. Provide a `tracing_subscriber::Layer` so femtologging handlers can
  process `tracing` spans and events. See
  [design §6.4](./rust-multithreaded-logging-framework-for-python-design.md#64-interoperability-with-the-rust-logging-ecosystem)
   and
  [Architectural Decision Record (ADR) 002 phase 2](./adr-002-journald-and-otel-support.md#phase-2--tracing-layer-for-opentelemetry-and-more).

### 3.4. Python exception payloads and macro ergonomics

- [x] 3.4.1. Add structured exception and stack payloads for `exc_info` and
  `stack_info`, including schema versioning and Python handler
  interoperability. See
  [ADR 001 decision](./adr-001-python-exception-logging.md#decision) and
  [ADR 001 schema versioning](./adr-001-python-exception-logging.md#schema-versioning).
- [ ] 3.4.2. Improve structured logging in macros to simplify context
  propagation. See
  [design §6.2](./rust-multithreaded-logging-framework-for-python-design.md#62-logging-macros-the-primary-user-interface)
   and
  [design §8.3](./rust-multithreaded-logging-framework-for-python-design.md#83-exploring-advanced-asynchronous-capabilities).

## 4. Verification, performance, and release readiness

### 4.1. Testing and benchmarking

- [x] 4.1.1. Establish baseline unit and integration testing across Rust and
  Python paths. See
  [design §8.1](./rust-multithreaded-logging-framework-for-python-design.md#81-suggested-implementation-roadmap)
   and
  [configuration design §7](./configuration-design.md#7-testing-and-benchmarking-coverage).
- [x] 4.1.2. Expand test coverage and start benchmarking for configuration and
  handler paths. See
  [configuration design §7](./configuration-design.md#7-testing-and-benchmarking-coverage)
   and
  [design §8.2](./rust-multithreaded-logging-framework-for-python-design.md#82-benchmarking-approach).
- [ ] 4.1.3. Benchmark directly against picologging and optimize hot paths based
  on measured bottlenecks. See
  [design §8.2](./rust-multithreaded-logging-framework-for-python-design.md#82-benchmarking-approach)
   and
  [design §5](./rust-multithreaded-logging-framework-for-python-design.md#5-optimizing-for-performance).
- [ ] 4.1.4. Provide unit and integration tests for all remaining roadmap
  features as they land (including timed rotation, tracing, and format-level
  compile-time filtering). See
  [design §8.1](./rust-multithreaded-logging-framework-for-python-design.md#81-suggested-implementation-roadmap)
   and [rstest guide](./rust-testing-with-rstest-fixtures.md).

### 4.2. Delivery and adoption

- [x] 4.2.1. Set up continuous integration to run Rust and Python quality gates.
  See [dev workflow §commands](./dev-workflow.md#commands) and
  [dev workflow §CI matrix](./dev-workflow.md#ci-compatibility-matrix).
- [ ] 4.2.2. Produce migration guidance for existing picologging adopters,
  including API parity limits and operational differences. See
  [design §6.1](./rust-multithreaded-logging-framework-for-python-design.md#61-balancing-api-familiarity-with-rust-idioms)
   and
  [design §9.2](./rust-multithreaded-logging-framework-for-python-design.md#92-alignment-with-user-requirements).
- [ ] 4.2.3. Publish the femtologging package and complete user-facing
  documentation updates for shipped capabilities. See
  [design §9.3](./rust-multithreaded-logging-framework-for-python-design.md#93-call-to-actionnext-steps)
   and [users guide](./users-guide.md).

## 5. Roadmap consolidation checks

### 5.1. Migration and deduplication validation

- [x] 5.1.1. Migrate tasks from both legacy roadmap documents into this single
  roadmap. See
  [roadmap style](./documentation-style-guide.md#roadmap-task-writing-guidelines).
- [x] 5.1.2. Merge duplicate tasks across legacy documents into one canonical
  task list (for example, compile-time filtering, dynamic reconfiguration, and
  tracing integration). See
  [roadmap formatting](./documentation-style-guide.md#roadmap-formatting-conventions).
- [x] 5.1.3. Re-evaluate historical outstanding tasks against current codebase
  state and mark completed work as done. See
  [dev workflow §commands](./dev-workflow.md#commands) and
  [rust extension](./rust-extension.md#rust-log-crate-bridge).
