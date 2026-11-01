//! Compile-pass UI test for detaching work that may block.
//!
//! The file-handler bindings hand the interpreter lock back with
//! `Python::detach` and then perform a bounded wait, so the closure they run
//! must own everything it touches. This pins that shape: a `Python<'_>` token, an
//! owned payload, and a wait performed entirely without the GIL.

use std::sync::mpsc;
use std::time::Duration;

use pyo3::prelude::*;

fn wait_once(py: Python<'_>) -> bool {
    let (tx, rx) = mpsc::channel::<()>();
    // No sender remains, so the receive fails at once; this fixture pins the
    // ownership of the receiver, not the channel's blocking behaviour.
    drop(tx);
    py.detach(move || rx.recv_timeout(Duration::from_millis(1)).is_ok())
}

#[pyfunction]
fn double_without_the_gil(py: Python<'_>, value: u32) -> PyResult<u32> {
    let doubled = py.detach(move || value * 2);
    Ok(doubled)
}

fn main() {}