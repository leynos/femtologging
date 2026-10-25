"""Overflow policy factory helpers exposed by the Rust extension.

A policy decides what a file handler does when its queue is full:

- ``OverflowPolicy.drop()`` discards the record and raises ``RuntimeError``.
  This is the default, and the only policy under which ``handle()`` never
  waits for queue capacity.
- ``OverflowPolicy.block()`` waits until the worker makes room. The wait is
  unbounded: nothing cancels it, so a worker that has stopped draining parks
  the caller for as long as that lasts.
- ``OverflowPolicy.timeout(ms)`` waits for up to ``ms`` milliseconds and then
  gives up and drops the record. ``FemtoFileHandler`` and
  ``FemtoRotatingFileHandler`` raise ``RuntimeError``; the timed-rotating
  binding maps the same enqueue failure to ``ValueError``.

The Python file-handler bindings release the interpreter lock for the duration
of every such wait, so another Python thread keeps running while one thread is
parked. The full contract, including what ``close()`` does to a call that is
already blocked and the separate waits in ``flush()`` and ``close()``, is
documented under "Concurrency and the GIL for file handlers" in
``docs/users-guide.md``.
"""

from __future__ import annotations

from ._femtologging_rs import OverflowPolicy

__all__ = ["OverflowPolicy"]
