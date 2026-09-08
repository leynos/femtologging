"""Tests for the FemtoFileHandler."""

from __future__ import annotations

import collections.abc as cabc
import errno
import re
import threading
import time
import typing as typ
from contextlib import AbstractContextManager, closing, contextmanager
from pathlib import Path

import pytest

from femtologging import FemtoFileHandler, FileHandlerBuilder, OverflowPolicy

type FileHandlerFactory = cabc.Callable[
    [Path, int, int], AbstractContextManager[FemtoFileHandler]
]

type LevelledRecord = tuple[str, str]
"""A ``(level, message)`` pair handled by the ``core`` logger in these tests."""


class FormatterRecord(typ.TypedDict):
    """Structured payload for the blocking formatter."""

    logger: str
    level: str
    message: str


def _rendered(records: cabc.Sequence[LevelledRecord]) -> list[str]:
    """Return the lines the default formatter produces for ``records``."""
    return [f"core [{level}] {message}" for level, message in records]


def _assert_log_lines(
    path: Path, expected: cabc.Sequence[str], context: str
) -> list[str]:
    """Assert the file holds exactly ``expected`` lines and return them."""
    actual = path.read_text().splitlines() if path.exists() else []
    assert actual == list(expected), (
        f"{context}: handler output diverged from the expected record sequence"
    )
    return actual


def _read_lines_with_retry(
    path: Path, expected: list[str], *, timeout: float = 1.0
) -> list[str]:
    """Read lines, retrying briefly to allow async flush to complete."""

    def read_lines() -> list[str]:
        return path.read_text().splitlines() if path.exists() else []

    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        lines = read_lines()
        if lines == expected:
            return lines
        time.sleep(0.01)
    return read_lines()


@pytest.mark.parametrize(
    ("handler_kwargs", "records"),
    [
        pytest.param(
            {},
            [("INFO", "first"), ("INFO", "second")],
            id="default-constructor-flushes-every-record",
        ),
        pytest.param(
            {"capacity": 8, "flush_interval": 1, "policy": "drop"},
            [("INFO", "hello")],
            id="single-record-reaches-disk",
        ),
        pytest.param(
            {"capacity": 8, "flush_interval": 1, "policy": "drop"},
            [("INFO", "first"), ("WARN", "second"), ("ERROR", "third")],
            id="mixed-levels-preserve-handling-order",
        ),
        pytest.param(
            {"capacity": 8, "flush_interval": 2, "policy": "drop"},
            [("INFO", "first"), ("INFO", "second"), ("INFO", "third")],
            id="buffered-flush-interval-still-writes-every-record",
        ),
        pytest.param(
            {"capacity": 2, "flush_interval": 1, "policy": "block"},
            [("INFO", "first"), ("INFO", "second"), ("INFO", "third")],
            id="block-policy-waits-rather-than-dropping",
        ),
        pytest.param(
            {"policy": " Drop "},
            [("INFO", "msg")],
            id="policy-string-normalized-for-case-and-whitespace",
        ),
    ],
)
def test_handled_records_persist_in_order(
    tmp_path: Path,
    handler_kwargs: dict[str, object],
    records: list[LevelledRecord],
) -> None:
    """Every handled record is written, in order, for each configuration."""
    path = tmp_path / "out.log"
    with closing(FemtoFileHandler(str(path), **handler_kwargs)) as handler:
        for level, message in records:
            handler.handle("core", level, message)
    _assert_log_lines(path, _rendered(records), f"configuration {handler_kwargs!r}")


def test_file_handler_concurrent_usage(
    tmp_path: Path, file_handler_factory: FileHandlerFactory
) -> None:
    """Concurrent writes should not lose messages."""
    path = tmp_path / "concurrent.log"
    with file_handler_factory(path, 10, 1) as handler:

        def send(h: FemtoFileHandler, i: int) -> None:
            h.handle("core", "INFO", f"msg{i}")

        threads = [threading.Thread(target=send, args=(handler, i)) for i in range(10)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
    written = set(path.read_text().splitlines())
    expected = set(_rendered([("INFO", f"msg{i}") for i in range(10)]))
    assert expected <= written, (
        f"concurrent handling dropped records: missing {sorted(expected - written)!r}"
    )


def test_file_handler_flush(tmp_path: Path) -> None:
    """Test that ``flush()`` writes pending records immediately."""
    path = tmp_path / "flush.log"
    with closing(FemtoFileHandler(str(path))) as handler:

        def send(msg: str) -> None:
            handler.handle("core", "INFO", msg)
            assert handler.flush() is True, (
                f"flush() must report success after handling {msg!r}"
            )

        send("one")
        _assert_log_lines(path, _rendered([("INFO", "one")]), "after first flush")
        send("two")
        _assert_log_lines(
            path,
            _rendered([("INFO", "one"), ("INFO", "two")]),
            "after second flush",
        )


def test_file_handler_flush_concurrent(
    tmp_path: Path, file_handler_factory: FileHandlerFactory
) -> None:
    """Concurrent ``flush()`` calls should each succeed."""
    path = tmp_path / "flush_concurrent.log"
    with file_handler_factory(path, 8, 1) as handler:

        def send_and_flush() -> None:
            handler.handle("core", "INFO", "msg")
            assert handler.flush() is True, (
                "concurrent flush() must report success for every caller"
            )

        threads = [threading.Thread(target=send_and_flush) for _ in range(5)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

    _assert_log_lines(path, _rendered([("INFO", "msg")] * 5), "five concurrent writers")


def test_file_handler_open_failure(tmp_path: Path) -> None:
    """Creating a handler in a missing directory raises ``OSError``."""
    bad_dir = tmp_path / "does_not_exist"
    path = bad_dir / "out.log"
    with pytest.raises(OSError, match=re.escape(str(path))) as excinfo:
        FemtoFileHandler(str(path))
    assert excinfo.value.errno in {None, errno.ENOENT}, (
        "a missing parent directory must surface as ENOENT (or an unset errno), "
        f"not {excinfo.value.errno!r}"
    )


def test_file_handler_flush_interval_large(tmp_path: Path) -> None:
    """Large flush_interval buffers records until the handler closes."""
    path = tmp_path / "large_flush.log"
    records: list[LevelledRecord] = [("INFO", f"msg {i}") for i in range(5)]
    with closing(
        FemtoFileHandler(
            str(path),
            capacity=8,
            flush_interval=10000,
            policy="drop",
        )
    ) as handler:
        for level, message in records:
            handler.handle("core", level, message)
        _assert_log_lines(path, [], "before close with a flush interval of 10000")
    _assert_log_lines(path, _rendered(records), "after close flushes the buffer")


def test_overflow_policy_timeout(tmp_path: Path) -> None:
    """Timeout policy drops records once the queue is saturated."""
    path = tmp_path / "timeout.log"
    worker_started = threading.Event()
    release_worker = threading.Event()

    @contextmanager
    def release_worker_on_exit(
        event: threading.Event,
    ) -> cabc.Generator[None, None, None]:
        try:
            yield
        finally:
            event.set()

    def blocking_formatter(record: FormatterRecord) -> str:
        if not worker_started.is_set():
            worker_started.set()
            if not release_worker.wait(timeout=10.0):
                msg = "timeout waiting for release_worker in formatter"
                raise AssertionError(msg)
        return f"{record['logger']} [{record['level']}] {record['message']}"

    builder = (
        FileHandlerBuilder(str(path))
        .with_capacity(1)
        .with_flush_after_records(10000)
        .with_overflow_policy(OverflowPolicy.timeout(200))
        .with_formatter(blocking_formatter)
    )
    # Release the worker before closing to avoid racing on the final flush.
    with closing(builder.build()) as handler, release_worker_on_exit(release_worker):
        handler.handle("core", "INFO", "first")
        assert worker_started.wait(10.0), "worker never reached formatter"
        # Capacity=1 allows one queued record while the worker is busy.
        handler.handle("core", "INFO", "second")
        with pytest.raises(RuntimeError, match="timed out"):
            handler.handle("core", "INFO", "third")
    expected = _rendered([("INFO", "first"), ("INFO", "second")])
    lines = _read_lines_with_retry(path, expected)
    assert lines == expected, "expected timeout policy to drop the third record"


_QUEUE_FULL_ERRORS = frozenset({
    "Handler error: queue full",
    "Handler error: handler is closed",
})


def _handle_tolerating_overflow(
    handler: FemtoFileHandler, level: str, message: str
) -> None:
    """Handle a record, allowing the documented overflow errors to surface."""
    error_msg: str | None = None
    try:
        handler.handle("core", level, message)
    except RuntimeError as err:
        error_msg = str(err)
    if error_msg is not None:
        assert error_msg in _QUEUE_FULL_ERRORS, (
            "drop policy must only reject records with a queue-full or "
            f"handler-closed error, got {error_msg!r}"
        )


@pytest.mark.parametrize(
    ("flush_interval", "record_count"),
    [
        pytest.param(1, 3, id="unbuffered-writes"),
        pytest.param(5, 10, id="buffered-writes"),
    ],
)
def test_overflow_policy_drop_keeps_earliest_records(
    tmp_path: Path, flush_interval: int, record_count: int
) -> None:
    """Drop policy discards excess records but preserves the earliest ones."""
    path = tmp_path / "drop.log"
    with closing(
        FemtoFileHandler(
            str(path),
            capacity=2,
            flush_interval=flush_interval,
            policy="drop",
        )
    ) as handler:
        for i in range(record_count):
            _handle_tolerating_overflow(handler, "INFO", f"msg{i}")
    # The consumer runs concurrently; on faster CI machines it may dequeue
    # between sends. Assert the first two messages are present in order,
    # without requiring later records to be dropped deterministically.
    leading = path.read_text().splitlines()[:2]
    assert leading == _rendered([("INFO", "msg0"), ("INFO", "msg1")]), (
        "drop policy must retain the earliest accepted records in order, "
        f"got {leading!r}"
    )


@pytest.mark.parametrize(
    ("handler_kwargs", "expected_message"),
    [
        pytest.param(
            {"capacity": 0},
            "capacity must be greater than zero",
            id="zero-capacity",
        ),
        pytest.param(
            {"flush_interval": 0},
            "flush_interval must be greater than zero",
            id="zero-flush-interval",
        ),
        pytest.param(
            {"flush_interval": -1},
            "flush_interval must be greater than zero",
            id="negative-flush-interval",
        ),
        pytest.param(
            {"policy": "bogus"},
            "invalid overflow policy",
            id="unknown-policy-name",
        ),
        pytest.param(
            {"policy": "timeout"},
            r"timeout requires a positive integer N, use 'timeout:N'",
            id="timeout-policy-without-duration",
        ),
        pytest.param(
            {"policy": "timeout:0"},
            "timeout must be greater than zero",
            id="timeout-policy-zero-duration",
        ),
        pytest.param(
            {"policy": "timeout:-1"},
            r"timeout must be a positive integer \(N in 'timeout:N'\)",
            id="timeout-policy-negative-duration",
        ),
        pytest.param(
            {"policy": "timeout:abc"},
            r"timeout must be a positive integer \(N in 'timeout:N'\)",
            id="timeout-policy-non-numeric-duration",
        ),
    ],
)
def test_constructor_rejects_invalid_configuration(
    tmp_path: Path, handler_kwargs: dict[str, object], expected_message: str
) -> None:
    """Invalid handler configuration is rejected with an explanatory error."""
    path = tmp_path / "invalid.log"
    with pytest.raises(ValueError, match=expected_message):
        FemtoFileHandler(str(path), **handler_kwargs)


def test_file_handler_handle_after_close_raises(tmp_path: Path) -> None:
    """Calling ``handle`` on a closed handler raises ``RuntimeError``."""
    path = tmp_path / "closed.log"
    handler = FemtoFileHandler(str(path))
    handler.close()
    with pytest.raises(RuntimeError, match="Handler error: handler is closed"):
        handler.handle("core", "INFO", "after close")
