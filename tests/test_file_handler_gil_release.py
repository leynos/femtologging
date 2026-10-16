"""The Python file-handler bindings must release the interpreter lock.

Issue #441: a direct ``handler.handle()`` call under a blocking overflow policy
waits for queue capacity while holding the GIL, so a slow or stalled file
worker starves every other Python thread. These tests pin the contract that
fixes it; ``tests/file_handler_gil_support.py`` holds the scenario harness that
runs each case in its own interpreter and explains how the verdict is reached.

PyO3's ``detach`` does not release the handler's borrow flag, so a blocked
``handle()`` keeps its shared borrow for the whole wait. A concurrent
``close()`` therefore fails to take the exclusive borrow it needs, which PyO3
reports as ``RuntimeError``; that scenario pins the resulting contract.
"""

from __future__ import annotations

import typing as typ

import pytest

from .file_handler_gil_scenarios import BUILDER_NAMES
from .file_handler_gil_support import run_scenario

if typ.TYPE_CHECKING:
    from pathlib import Path


@pytest.mark.concurrency
@pytest.mark.parametrize("builder_name", BUILDER_NAMES)
def test_blocking_handle_releases_the_gil(tmp_path: Path, builder_name: str) -> None:
    """An unbounded ``"block"`` wait must not stop a second Python thread."""
    path = tmp_path / f"{builder_name}-block.log"
    run_scenario(builder_name, "handle-block", path)
    # Every record the wait accepted is written once the worker drains.
    assert path.read_text().splitlines() == [
        "core [INFO] first",
        "core [INFO] second",
        "core [INFO] third",
    ], "a released block-policy handler must write every accepted record in order"


@pytest.mark.concurrency
@pytest.mark.parametrize("builder_name", BUILDER_NAMES)
def test_timed_handle_releases_the_gil(tmp_path: Path, builder_name: str) -> None:
    """A bounded ``"timeout:N"`` wait must not stop a second Python thread."""
    path = tmp_path / f"{builder_name}-timeout.log"
    run_scenario(builder_name, "handle-timeout", path)
    # The record that timed out is dropped; the accepted ones still persist.
    assert path.read_text().splitlines() == [
        "core [INFO] first",
        "core [INFO] second",
    ], "a timed-out handle() must drop only the record it could not enqueue"


@pytest.mark.concurrency
def test_blocking_flush_releases_the_gil(tmp_path: Path) -> None:
    """``flush()`` waiting on a parked worker must release the GIL."""
    run_scenario("file", "flush", tmp_path / "flush.log")


@pytest.mark.concurrency
def test_blocking_close_releases_the_gil(tmp_path: Path) -> None:
    """``close()`` waiting on a parked worker must release the GIL."""
    run_scenario("file", "close", tmp_path / "close.log")


@pytest.mark.concurrency
def test_close_while_handle_is_blocked_raises_runtime_error(tmp_path: Path) -> None:
    """A concurrent ``close()`` must fail cleanly rather than corrupt state."""
    run_scenario("file", "concurrent-close", tmp_path / "concurrent-close.log")
