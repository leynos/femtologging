# Documentation Contents

This directory holds design notes for `femtologging` and references to relevant
libraries. Use the links below to explore each topic.

## Setup and Workflow

- [dev-workflow.md](./dev-workflow.md)
  - Describes Makefile commands for building, linting, formatting and tests.
- [dependency-analysis.md](./dependency-analysis.md)
  - Summarizes third-party crates chosen for the Rust implementation.
- [documentation-style-guide.md](./documentation-style-guide.md)
  - Provides conventions for documentation and Python docstrings.
- [roadmap.md](./roadmap.md)
  - Lists milestones for porting picologging to a Rust/PyO3 implementation.

## Architecture decision records

- [adr-001-python-exception-logging.md](./adr-001-python-exception-logging.md)
  - Proposes support for `exc_info` and `stack_info` in the Python logging API.

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
