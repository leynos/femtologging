"""Overflow policy factory helpers exposed by the Rust extension.

A policy decides what a file handler does when its queue is full:

- ``OverflowPolicy.drop()`` discards the record and raises ``RuntimeError``.
  This is the default, and the only policy under which a handler operation
  never waits.
- ``OverflowPolicy.block()`` waits until the worker makes room. The wait is
  unbounded: nothing cancels it, so a worker that has stopped draining parks
  the caller for as long as that lasts.
- ``OverflowPolicy.timeout(ms)`` waits for up to ``ms`` milliseconds and then
  gives up, raising ``RuntimeError`` and dropping the record.

The Python file-handler bindings release the interpreter lock for the duration
of every such wait, so another Python thread keeps running while one thread is
parked. The full contract, including what ``close()`` does to a call that is
already blocked, is documented under "Concurrency and the GIL for file
handlers" in ``docs/users-guide.md``.
"""

from __future__ import annotations

from ._femtologging_rs import OverflowPolicy

__all__ = ["OverflowPolicy"]
