# add_python_bindings entry point

`add_python_bindings` registers Python-only builders and errors with the
`_femtologging_rs` module. It keeps conditional compilation concise by grouping
these registrations in one place.

The function is available only when the `python` feature is enabled:

```rust
#[pymodule]
fn _femtologging_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    #[cfg(feature = "python")]
    add_python_bindings(m)?;
    Ok(())
}
```

The crate re-exports the registered types, so Rust code can construct them
directly:

```rust
use femtologging_rs::{ConfigBuilder, LevelFilterBuilder};
```

See [rust-extension.md](./rust-extension.md#public-api-re-exports) for a list
of the re-exported symbols.
