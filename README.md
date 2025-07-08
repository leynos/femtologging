# femtologging

Example package generated from this Copier template.

This variant includes a small Rust extension built with [PyO3](https://pyo3.rs/)
and packaged using [maturin](https://maturin.rs/). Ensure the
[Rust tool‑chain](https://www.rust-lang.org/tools/install) is installed, then
run `pip install .` or `make build` to compile the extension.

## Usage

```python
from femtologging import FemtoFileHandler

handler = FemtoFileHandler.with_capacity_flush_timeout(
    "app.log", capacity=32, flush_interval=1, timeout_ms=200
)
handler.handle("core", "INFO", "hello")
handler.close()
```
