# Roadmap: Port picologging to Rust/PyO3

The design document in
[`rust-multithreaded-logging-framework-for-python-design.md`](./rust-multithreaded-logging-framework-for-python-design.md)
outlines a phased approach for building `femtologging`. The high‑level goal is
to re‑implement picologging in Rust with strong compile‑time safety and a
multithreaded handler model. The steps below summarise the actionable items from
that design.

## Initial Setup Tasks

- [x] Review picologging codebase and isolate core logging features
- [x] Evaluate dependencies and identify Rust equivalents
- [x] Design Rust crate layout and expose PyO3 bindings
- [x] Implement basic logger in Rust with matching Python API
- [x] Integrate Rust extension into Python packaging workflow
- [ ] Port formatting and handler components to Rust
- [ ] Add concurrency support and thread safety guarantees
- [ ] Benchmark against picologging and optimise hot paths
- [ ] Provide unit and integration tests for all features
- [ ] Set up continuous integration for Rust and Python tests
- [ ] Write migration guide for existing picologging users
- [ ] Publish femtologging package and update documentation

## Phase 1 – Core Functionality & Minimum Viable Product

- [ ] Define the `FemtoLogRecord` structure and implement core `FemtoLogger`
  logic, including efficient level checking and logging macros.
- [ ] Implement the `FemtoFormatter` trait with a default formatter.
- [ ] Select and integrate an MPSC channel for producer‑consumer queues.
- [ ] Create `FemtoStreamHandler` and `FemtoFileHandler`, each running in a
  dedicated consumer thread.
- [ ] Provide a programmatic configuration API using the builder pattern.
- [ ] Add compile‑time log level filtering via Cargo features.
- [ ] Ensure all components satisfy `Send`/`Sync` requirements.
- [ ] Establish a basic test suite covering unit and integration tests.

## Phase 2 – Expanded Handlers & Core Features

- [ ] Implement `FemtoRotatingFileHandler` and `FemtoTimedRotatingFileHandler`
  with their respective rotation logic.
- [ ] Add `FemtoSocketHandler` with serialization (e.g. MessagePack or CBOR) and
  reconnection handling.
- [ ] Define the `FemtoFilter` trait and implement common filter types.
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
- [ ] Explore batching optimisations in consumer threads.

These phases will lead to a robust Rust implementation that matches the
performance goals of picologging while leveraging Rust's safety guarantees.
Development should start with Phase&nbsp;1 to deliver a minimal, testable
product and iterate from there.
