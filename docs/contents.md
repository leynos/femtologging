# Documentation Contents

This directory holds design notes for `femtologging` and references to
relevant libraries. Use the links below to explore each topic.

- [concurrency-models-in-high-performance-logging.md](./concurrency-models-in-high-performance-logging.md)
  - Examines picologging's hybrid lock strategy and contrasts it with
    Rust's compile-time safety and asynchronous patterns.
- [core_features.md](./core_features.md)
  - Summarizes picologging's key features prioritized for the Rust port.
- [logging-class-overview.md](./logging-class-overview.md)
  - Presents a Mermaid class diagram showing CPython logging's hierarchy.
- [logging-cpython-picologging-comparison.md](./logging-cpython-picologging-comparison.md)
  - Compares CPython logging and picologging implementations and
    performance trade-offs.
- [logging-sequence-diagrams.md](./logging-sequence-diagrams.md)
  - Contains sequence diagrams illustrating logging call flows.
- [roadmap.md](./roadmap.md)
  - Lists milestones for porting picologging to a Rust/PyO3 implementation.
- [rust-extension.md](./rust-extension.md)
  - Describes the small PyO3-based Rust extension shipped in the project.
- [rust-multithreaded-logging-framework-for-python-design.md](./rust-multithreaded-logging-framework-for-python-design.md)
  - Proposes a multithreaded Rust logging framework with strong
    compile-time safety.
- [rust-testing-with-rstest-fixtures.md](./rust-testing-with-rstest-fixtures.md)
  - Explains how to use the `rstest` crate for fixture-based tests.
- [dependency-analysis.md](./dependency-analysis.md)
  - Summarises third-party crates chosen for the Rust implementation.
