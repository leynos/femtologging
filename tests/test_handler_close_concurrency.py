"""Concurrency contracts for Python handler flush and close calls."""

from __future__ import annotations

import dataclasses
import threading
import time
import typing as typ

import pytest

from femtologging import (
    FemtoFileHandler,
    FemtoStreamHandler,
    FileHandlerBuilder,
    StreamHandlerBuilder,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    import pathlib


type Handler = FemtoFileHandler | FemtoStreamHandler


@dataclasses.dataclass(frozen=True, slots=True)
class BlockedHandler:
    """Handler whose worker is waiting inside a Python formatter."""

    handler: Handler
    formatter_entered: threading.Event
    release_formatter: threading.Event


@pytest.fixture(params=("file", "stream"))
def blocked_handler(
    request: pytest.FixtureRequest, tmp_path: pathlib.Path
) -> cabc.Iterator[BlockedHandler]:
    """Build a file or stream handler paused in its formatter callback."""
    formatter_entered = threading.Event()
    release_formatter = threading.Event()

    def pause_formatter(_record: cabc.Mapping[str, object]) -> str:
        """Signal that the worker entered Python, then wait for release."""
        formatter_entered.set()
        if not release_formatter.wait(timeout=5):
            raise TimeoutError
        return "paused record"

    if request.param == "file":
        handler = (
            FileHandlerBuilder(str(tmp_path / "blocked.log"))
            .with_formatter(pause_formatter)
            .build()
        )
    else:
        handler = StreamHandlerBuilder.stderr().with_formatter(pause_formatter).build()

    handler.handle("concurrency", "INFO", "pause worker")
    try:
        assert formatter_entered.wait(timeout=2), "worker did not enter formatter"
        yield BlockedHandler(handler, formatter_entered, release_formatter)
    finally:
        release_formatter.set()
        handler.close()


def test_python_flush_releases_gil_while_worker_waits(
    blocked_handler: BlockedHandler,
) -> None:
    """Another Python thread can run while ``flush`` awaits the worker."""
    flush_started = threading.Event()
    flush_finished = threading.Event()
    python_thread_ran = threading.Event()
    flush_results: list[bool] = []
    flush_errors: list[str] = []
    progress_thread: threading.Thread | None = None

    def flush() -> None:
        """Flush from a worker thread while the formatter is blocked."""
        flush_started.set()
        try:
            flush_results.append(blocked_handler.handler.flush())
        except RuntimeError as error:
            flush_errors.append(str(error))
        finally:
            flush_finished.set()

    worker = threading.Thread(target=flush)
    worker.start()
    try:
        assert flush_started.wait(timeout=1), "flush thread did not start"
        assert not flush_finished.wait(timeout=0.1), "flush did not wait for the worker"
        progress_thread = threading.Thread(target=python_thread_ran.set)
        progress_thread.start()
        assert python_thread_ran.wait(timeout=0.2), (
            "another Python thread could not run while flush waited"
        )
        progress_thread.join(timeout=1)
        blocked_handler.handler.handle("concurrency", "INFO", "Python thread ran")
        assert not flush_finished.is_set(), (
            "flush finished before the worker was released"
        )
    finally:
        blocked_handler.release_formatter.set()
        worker.join(timeout=2)
        if progress_thread is not None:
            progress_thread.join(timeout=1)

    assert not worker.is_alive(), "flush thread did not finish"
    assert not flush_errors, f"flush raised: {flush_errors!r}"
    assert flush_results == [True], "flush did not receive the worker acknowledgement"
    assert progress_thread is not None, "Python progress thread was not started"
    assert not progress_thread.is_alive(), "Python progress thread did not finish"


def test_python_close_can_run_while_flush_waits(
    blocked_handler: BlockedHandler,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Close can borrow the handler while a detached flush waits."""
    flush_started = threading.Event()
    flush_finished = threading.Event()
    close_started = threading.Event()
    close_finished = threading.Event()
    flush_results: list[bool] = []
    thread_errors: list[str] = []
    close_thread: threading.Thread | None = None

    def record_thread_error(args: threading.ExceptHookArgs) -> None:
        """Capture uncaught thread exceptions, including PyO3 panic errors."""
        thread_errors.append(f"{args.exc_type.__name__}: {args.exc_value}")

    monkeypatch.setattr(threading, "excepthook", record_thread_error)

    def flush() -> None:
        """Flush from a thread while the handler worker is blocked."""
        flush_started.set()
        try:
            flush_results.append(blocked_handler.handler.flush())
        finally:
            flush_finished.set()

    def close() -> None:
        """Close from another thread while the flush is still waiting."""
        close_started.set()
        try:
            blocked_handler.handler.close()
        finally:
            close_finished.set()

    flush_thread = threading.Thread(target=flush)
    flush_thread.start()
    try:
        assert flush_started.wait(timeout=1), "flush thread did not start"
        assert not flush_finished.wait(timeout=0.1), "flush did not wait for the worker"
        close_thread = threading.Thread(target=close)
        close_thread.start()
        assert close_started.wait(timeout=1), "close thread did not start"
        assert not close_finished.wait(timeout=0.1), (
            "close failed while flush held a shared PyO3 borrow"
        )
    finally:
        blocked_handler.release_formatter.set()
        flush_thread.join(timeout=2)
        if close_thread is not None:
            close_thread.join(timeout=2)

    assert not flush_thread.is_alive(), "flush thread did not finish"
    assert close_thread is not None, "close thread was not started"
    assert not close_thread.is_alive(), "close thread did not finish"
    assert not thread_errors, f"handler calls raised: {thread_errors!r}"
    assert flush_results == [True], "flush did not receive the worker acknowledgement"


def test_python_close_releases_pyclass_borrow_while_worker_waits(
    blocked_handler: BlockedHandler,
) -> None:
    """Other Python threads can progress and borrow while ``close`` waits."""
    close_started = threading.Event()
    close_finished = threading.Event()
    python_thread_ran = threading.Event()
    close_thread_errors: list[str] = []
    handler_errors: list[str] = []
    progress_thread: threading.Thread | None = None

    def close() -> None:
        """Close from a worker thread while the formatter is blocked."""
        close_started.set()
        try:
            blocked_handler.handler.close()
        except RuntimeError as error:
            close_thread_errors.append(str(error))
        finally:
            close_finished.set()

    worker = threading.Thread(target=close)
    close_call_started_at = time.monotonic()
    worker.start()
    try:
        assert close_started.wait(timeout=1), "close thread did not start"
        deadline = time.monotonic() + 2
        while True:
            try:
                blocked_handler.handler.handle("concurrency", "INFO", "probe")
            except RuntimeError as error:
                handler_errors.append(str(error).lower())
                break
            assert not close_finished.is_set(), (
                "close returned before the worker was released"
            )
            assert time.monotonic() < deadline, (
                "close did not begin shutting down the handler"
            )
            time.sleep(0.005)
        assert not close_finished.is_set(), (
            "close finished while the worker remained blocked"
        )
        assert time.monotonic() - close_call_started_at < 0.75, (
            "close held the GIL until its worker wait timed out"
        )
        progress_thread = threading.Thread(target=python_thread_ran.set)
        progress_thread.start()
        assert python_thread_ran.wait(timeout=0.2), (
            "another Python thread could not run while close waited"
        )
    finally:
        blocked_handler.release_formatter.set()
        worker.join(timeout=2)
        if progress_thread is not None:
            progress_thread.join(timeout=1)

    assert not worker.is_alive(), "close thread did not finish"
    assert progress_thread is not None, "Python progress thread was not started"
    assert not progress_thread.is_alive(), "Python progress thread did not finish"
    assert not close_thread_errors, f"close raised: {close_thread_errors!r}"
    assert len(handler_errors) == 1, "handler did not report its closed state"
    assert "already borrowed" not in handler_errors[0], (
        "close retained the PyO3 mutable borrow while waiting"
    )
    assert "closed" in handler_errors[0], "unexpected handler error"


def test_every_concurrent_close_waits_for_the_worker(
    blocked_handler: BlockedHandler,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A later ``close`` waits for the worker the first ``close`` is joining.

    Only the first closer can take the join handle, so a second closer that
    returned as soon as the handle was gone would report a completed shutdown
    while the worker was still writing queued records. An application that
    closed the handler from two threads and then tore down the destination
    could lose those records.
    """
    first_started = threading.Event()
    first_finished = threading.Event()
    second_started = threading.Event()
    second_finished = threading.Event()
    thread_errors: list[str] = []
    first_thread: threading.Thread | None = None
    second_thread: threading.Thread | None = None

    def record_thread_error(args: threading.ExceptHookArgs) -> None:
        """Capture uncaught thread exceptions, including PyO3 panic errors."""
        thread_errors.append(f"{args.exc_type.__name__}: {args.exc_value}")

    monkeypatch.setattr(threading, "excepthook", record_thread_error)

    def close(started: threading.Event, finished: threading.Event) -> None:
        """Close the handler and report when the call returned."""
        started.set()
        try:
            blocked_handler.handler.close()
        finally:
            finished.set()

    first_thread = threading.Thread(target=close, args=(first_started, first_finished))
    first_thread.start()
    try:
        assert first_started.wait(timeout=1), "first close thread did not start"
        assert not first_finished.wait(timeout=0.1), (
            "first close did not wait for the blocked worker"
        )
        second_thread = threading.Thread(
            target=close, args=(second_started, second_finished)
        )
        second_thread.start()
        assert second_started.wait(timeout=1), "second close thread did not start"
        assert not second_finished.wait(timeout=0.2), (
            "a concurrent close returned before the worker finished"
        )
    finally:
        blocked_handler.release_formatter.set()
        first_thread.join(timeout=5)
        if second_thread is not None:
            second_thread.join(timeout=5)

    assert not first_thread.is_alive(), "first close thread did not finish"
    assert second_thread is not None, "second close thread was not started"
    assert not second_thread.is_alive(), "second close thread did not finish"
    assert first_finished.is_set(), "first close never observed completion"
    assert second_finished.is_set(), "second close never observed completion"
    assert not thread_errors, f"handler calls raised: {thread_errors!r}"
