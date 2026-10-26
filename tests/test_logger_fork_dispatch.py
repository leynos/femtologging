"""Dispatch-pool process-boundary tests.

Purpose
-------
Verify that a child process created by ``fork`` rebinds inherited logger
instances to its own workers and starts the same pool for newly created
loggers.
"""

from __future__ import annotations

import gc
import os
import typing as typ
import warnings

import pytest

from femtologging import get_logger, reset_manager

if typ.TYPE_CHECKING:
    import collections.abc as cabc


class _PipeHandler:
    """Write handled messages to the pipe shared with the parent."""

    def __init__(self, write_fd: int) -> None:
        self.write_fd = write_fd

    def handle(self, logger_name: str, _level: str, message: str) -> None:
        os.write(self.write_fd, f"{logger_name}:{message}\n".encode())


class _Loggable(typ.Protocol):
    """The logger methods the child process exercises."""

    def info(self, message: str) -> None: ...

    def flush_handlers(self) -> bool: ...


def _run_child_process(
    parent_logger: _Loggable,
    handler: _PipeHandler,
    read_fd: int,
    write_fd: int,
) -> None:
    os.close(read_fd)
    try:
        # The inherited logger must rebind its state, and construction must use
        # the same child pool when it creates another logger.
        parent_logger.info("inherited logger delivered")
        inherited_flushed = parent_logger.flush_handlers()
        child_logger = get_logger("fork.child")
        child_logger.add_handler(handler)
        child_logger.info("new logger delivered")
        new_logger_flushed = child_logger.flush_handlers()
        os.write(
            write_fd,
            f"flush:{inherited_flushed}:{new_logger_flushed}\n".encode(),
        )
        os._exit(0 if inherited_flushed and new_logger_flushed else 1)
    except (OSError, RuntimeError, TypeError, ValueError) as error:
        os.write(write_fd, f"error:{error!r}\n".encode())
        os._exit(1)


@pytest.fixture(autouse=True)
def _clean_registry() -> cabc.Iterator[None]:
    """Start and end the fork test with an empty registry."""
    reset_manager()
    yield
    reset_manager()
    gc.collect()


@pytest.mark.skipif(not hasattr(os, "fork"), reason="fork is only available on POSIX")
def test_a_child_process_rebinds_inherited_and_new_loggers() -> None:
    """A child must not reuse its parent's queues after their workers vanish."""
    # Creating this logger initializes the parent's shared pool before the fork.
    parent_logger = get_logger("fork.parent")
    read_fd, write_fd = os.pipe()
    handler = _PipeHandler(write_fd)
    parent_logger.add_handler(handler)
    # CPython warns only when multiple Python-managed threads are active; its
    # warning condition does not reliably count the dispatcher's Rust workers.
    # Capture the optional warning so a Python-threaded test runner stays quiet,
    # while checking any warning is the expected consequence of this fork.
    with warnings.catch_warnings(record=True) as fork_warnings:
        warnings.simplefilter("always", DeprecationWarning)
        child_pid = os.fork()
    assert all(
        warning.category is DeprecationWarning
        and "multi-threaded, use of fork() may lead to deadlocks"
        in str(warning.message)
        for warning in fork_warnings
    ), "any fork warning must be CPython's multi-threaded fork warning"

    if child_pid == 0:
        try:
            _run_child_process(parent_logger, handler, read_fd, write_fd)
        finally:
            os._exit(1)

    os.close(write_fd)
    try:
        waited_pid, wait_status = os.waitpid(child_pid, 0)
        output = bytearray()
        while chunk := os.read(read_fd, 4096):
            output.extend(chunk)
    finally:
        os.close(read_fd)

    assert waited_pid == child_pid, "the expected child process must be reaped"
    assert os.waitstatus_to_exitcode(wait_status) == 0, output.decode(errors="replace")
    assert b"fork.parent:inherited logger delivered\n" in output, (
        "a logger inherited from the parent must rebind to the child's worker"
    )
    assert b"fork.child:new logger delivered\n" in output, (
        "a logger constructed in the child must use the child's worker"
    )
    assert b"flush:True:True\n" in output, "the child's workers must answer flush"
