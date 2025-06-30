# femtologging

Example package generated from this Copier template.

This variant includes a small Rust extension built with [PyO3](https://pyo3.rs/)
and packaged using [maturin](https://maturin.rs/). Ensure the
[Rust tool‑chain](https://www.rust-lang.org/tools/install) is installed, then
run `pip install .` or `make build` to compile the extension. The extension now
exposes an `OverflowPolicy` enum and the
`FemtoFileHandler::with_capacity_policy()` helper for controlling how the log
queue handles overflow situations.
