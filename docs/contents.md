# Documentation Contents

This directory holds design notes for `femtologging` and references to relevant
libraries. Use the links below to explore each topic.

## Setup and Workflow

- [dev-workflow.md](./dev-workflow.md)
  - Describes Makefile commands for building, linting, formatting and tests.
- [developers-guide.md](./developers-guide.md)
  - Records developer toolchain ownership and validation boundaries.
- [v0-2-0-migration-guide.md](./v0-2-0-migration-guide.md)
  - Describes structured logging additions and the bounded logger and shared
    dispatch changes in v0.2.0.
- [dependency-analysis.md](./dependency-analysis.md)
  - Summarizes third-party crates chosen for the Rust implementation.
- [documentation-style-guide.md](./documentation-style-guide.md)
  - Provides conventions for documentation and Python docstrings.
- [roadmap.md](./roadmap.md)
  - Lists milestones for porting picologging to a Rust/PyO3 implementation.

## Migration guides

- [v0-1-0-migration-guide.md](./v0-1-0-migration-guide.md)
  - Records the v0.1.0 renames from British to Oxford spelling across the
    Rust modules and the Python surface.
- [v0-2-0-migration-guide.md](./v0-2-0-migration-guide.md)
  - Describes the `BasicConfig` slots, bounded logger names and registry, and
    shared dispatch changes in v0.2.0 with migration guidance.

## Architecture decision records

- [adrs/adr-0001-python-exception-logging.md](./adrs/adr-0001-python-exception-logging.md)
  - Proposes support for `exc_info` and `stack_info` in the Python logging API.
- [adrs/adr-0002-journald-and-otel-support.md](./adrs/adr-0002-journald-and-otel-support.md)
  - Records the phased design for Journald support and OpenTelemetry
    integration via `tracing`.
- [adrs/adr-0003-python-stdlib-filter-parity.md](./adrs/adr-0003-python-stdlib-filter-parity.md)
  - Records the decision to add phased parity for Python stdlib callback
    filters.
- [adrs/adr-0004-batching-optimizations-in-consumer-threads.md](./adrs/adr-0004-batching-optimizations-in-consumer-threads.md)
  - Proposes drain-loop batching and vectored I/O optimizations for handler
    consumer threads.
- [adrs/adr-0005-four-tier-python-lint-architecture.md](./adrs/adr-0005-four-tier-python-lint-architecture.md)
  - Records the Python lint pipeline: Ruff, interrogate, PyPy-backed Pylint,
    df12-python-lints/ambrleaks, and the strict Skylos dead-code gate.
- [adrs/adr-0006-environment-seam-taxonomy.md](./adrs/adr-0006-environment-seam-taxonomy.md)
  - Records the ban on ambient process-environment access in the Rust
    extension and the injection shapes that replace it.
- [adrs/adr-0007-bounded-logger-cardinality-and-shared-dispatch.md](./adrs/adr-0007-bounded-logger-cardinality-and-shared-dispatch.md)
  - Records the bounds on logger names and registry size, and the shared
    dispatch pool that replaced one worker thread per logger.
- [adr-008-release-pipeline.md](./adr-008-release-pipeline.md)
  - Records the attested release pipeline, production feature set and
    idempotent PyPI/GitHub Release reconciliation.

## Logging Architecture

- [logging-class-overview.md](./logging-class-overview.md)
  - Presents a Mermaid class diagram showing CPython logging's hierarchy.
- <!-- markdownlint-disable-next-line MD013 -->
- [logging-cpython-picologging-comparison.md](./logging-cpython-picologging-
  comparison.md)
  - Compares CPython logging and picologging implementations and performance
    trade-offs.
- [logging-sequence-diagrams.md](./logging-sequence-diagrams.md)
  - Contains sequence diagrams illustrating logging call flows.

## Rust Port Design Notes

- <!-- markdownlint-disable-next-line MD013 -->
- [concurrency-models-in-high-performance-logging.md](./concurrency-models-in-
  high-performance-logging.md)
  - Examines picologging's hybrid lock strategy and contrasts it with Rust's
    compile-time safety and asynchronous patterns.
- [core_features.md](./core_features.md)
  - Summarizes picologging's key features prioritized for the Rust port.
- [formatters-and-handlers-rust-port.md](./formatters-and-handlers-rust-port.md)
  - Design for moving formatter and handler logic to Rust with thread safety.
- <!-- markdownlint-disable-next-line MD013 -->
- [logger-hierarchy-and-multi-handler.md](./logger-hierarchy-and-multi-
  handler.md)
  - Describes how loggers share handlers and inherit configuration via dotted
    names.
- [rust-extension.md](./rust-extension.md)
  - Describes the small PyO3-based Rust extension shipped in the project.
- [add-python-bindings.md](./add-python-bindings.md)
  - Explains the `add_python_bindings` entry point and its feature gating.
- <!-- markdownlint-disable-next-line MD013 -->
- [cpython-abi-management-in-rust-with-pyo3.md](./cpython-abi-management-in-
  rust-with-pyo3.md)
  - Comprehensive guide to managing CPython ABI compatibility with PyO3.
- <!-- markdownlint-disable-next-line MD013 -->
- [rust-multithreaded-logging-framework-for-python-design.md](./rust-
  multithreaded-logging-framework-for-python-design.md)
  - Proposes a multithreaded Rust logging framework with strong compile-time
    safety.
- [rust-testing-with-rstest-fixtures.md](./rust-testing-with-rstest-fixtures.md)
  - Explains how to use the `rstest` crate for fixture-based tests.
