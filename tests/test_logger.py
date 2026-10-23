"""Core behaviour tests for :class:`FemtoLogger`.

Covers message formatting, level parsing and filtering, handler
registration validation, and handler lifecycle. Exception and stack
capture live in ``tests/test_logger_exc_info.py``; handler dispatch
capability freezing lives in ``tests/test_logger_dispatch.py``.
"""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import FemtoLogger
from tests.logger_support import CollectingHandler

if typ.TYPE_CHECKING:
    from pathlib import Path

    from tests.conftest import FileHandlerFactory

LEVELS = ("TRACE", "DEBUG", "INFO", "WARN", "ERROR", "CRITICAL")


@pytest.mark.parametrize(
    ("name", "level", "message", "expected"),
    [
        ("core", "INFO", "hello", "core [INFO] hello"),
        ("sys", "ERROR", "fail", "sys [ERROR] fail"),
        ("core", "INFO", "", "core [INFO] "),
        ("i18n", "INFO", "こんにちは世界", "i18n [INFO] こんにちは世界"),
        ("api.v1.widgets", "INFO", "hello", "api.v1.widgets [INFO] hello"),
    ],
)
def test_log_formats_message(
    name: str, level: str, message: str, expected: str
) -> None:
    """Logger.log should format messages using configured formatter."""
    logger = FemtoLogger(name)
    assert logger.log(level, message) == expected, (
        f"unexpected format for name={name!r}, level={level!r}, message={message!r}"
    )


def test_a_long_message_is_formatted_in_full() -> None:
    """Message length is not part of the logger-name bound.

    This used to ride along in the message-formatting table with a 1000-byte
    logger name, which the name bound now refuses. A message has no length
    limit, so the case is kept — with a legal name — so the formatting path is
    still exercised at size.
    """
    logger = FemtoLogger("core")
    message = "m" * 1000
    assert logger.log("INFO", message) == f"core [INFO] {message}", (
        "a 1000-byte message must be formatted in full; the logger-name bound "
        "does not truncate messages"
    )


def test_an_over_long_logger_name_is_refused() -> None:
    """The byte bound is on the name, and the diagnostic says so.

    A name is a registry key that lives for the life of the process, so its
    length is a resource bound rather than formatting preference. The refusal
    has to name the bound to be actionable: a caller told only "invalid" would
    keep trimming by guesswork.
    """
    with pytest.raises(ValueError, match="cannot exceed 255 bytes"):
        FemtoLogger("n" * 1000)


def test_log_respects_logger_level() -> None:
    """Messages below the logger level should be ignored."""
    logger = FemtoLogger("core")
    logger.set_level("ERROR")
    assert logger.log("INFO", "ignored") is None, (
        "INFO must be suppressed while the logger level is ERROR"
    )
    assert logger.log("ERROR", "processed") == "core [ERROR] processed", (
        "ERROR must be emitted while the logger level is ERROR"
    )


@pytest.mark.parametrize("level", LEVELS)
def test_log_emits_at_own_level(level: str) -> None:
    """A record at the logger's own level should always be emitted."""
    logger = FemtoLogger("core")
    logger.set_level(level)
    assert logger.log(level, "ok") is not None, (
        f"record at the logger's own level {level!r} must be emitted"
    )


def test_log_filters_below_level_and_rejects_unknown_level() -> None:
    """Sub-threshold records are dropped and unknown levels are rejected."""
    logger = FemtoLogger("core")
    logger.set_level("ERROR")
    assert logger.log("WARN", "drop") is None, (
        "WARN must be dropped while the logger level is ERROR"
    )
    with pytest.raises(ValueError, match="level"):
        logger.log("bogus", "drop")


def test_logger_drop_no_hang(
    tmp_path: Path, file_handler_factory: FileHandlerFactory
) -> None:
    """FemtoLogger cleanup shouldn't block waiting on its thread."""
    path1 = tmp_path / "one.log"
    path2 = tmp_path / "two.log"
    with (
        file_handler_factory(path1, 8, 1) as h1,
        file_handler_factory(path2, 8, 1) as h2,
    ):
        logger = FemtoLogger("core")
        logger.add_handler(h1)
        logger.add_handler(h2)
        logger.log("INFO", "hello")
        del logger
    assert path1.read_text() == "core [INFO] hello\n", (
        f"first handler did not receive the record: {path1.read_text()!r}"
    )
    assert path2.read_text() == "core [INFO] hello\n", (
        f"second handler did not receive the record: {path2.read_text()!r}"
    )


def test_add_handler_requires_handle() -> None:
    """Adding a handler requires a callable handle attribute."""
    logger = FemtoLogger("core")

    class MissingHandle:
        pass

    with pytest.raises(TypeError, match="callable 'handle' method"):
        logger.add_handler(MissingHandle())

    class NotCallable:
        handle = "oops"

    with pytest.raises(TypeError, match="not callable"):
        logger.add_handler(NotCallable())


@pytest.mark.parametrize(
    ("emitted", "expected"),
    [
        ([("INFO", "ok")], [("core", "INFO", "ok")]),
        (
            [("INFO", "first"), ("ERROR", "second")],
            [("core", "INFO", "first"), ("core", "ERROR", "second")],
        ),
    ],
    ids=["single_record", "multiple_records"],
)
def test_python_handler_invocation(
    emitted: list[tuple[str, str]], expected: list[tuple[str, str, str]]
) -> None:
    """Python handlers should receive every emitted record in order."""
    logger = FemtoLogger("core")
    collector = CollectingHandler()
    logger.add_handler(collector)
    for level, message in emitted:
        logger.log(level, message)
    del logger
    assert collector.records == expected, (
        f"handler received {collector.records!r}, expected {expected!r}"
    )


@pytest.mark.parametrize("level", LEVELS)
def test_level_getter_returns_current_level(level: str) -> None:
    """Level property should reflect changes from set_level."""
    logger = FemtoLogger("core")
    logger.set_level(level)
    assert logger.level == level, (
        f"level getter returned {logger.level!r} after set_level({level!r})"
    )


def test_level_default_is_info() -> None:
    """New loggers should default to INFO level."""
    logger = FemtoLogger("core")
    assert logger.level == "INFO", (
        f"new logger defaulted to {logger.level!r}, expected 'INFO'"
    )


def test_set_level_invalid_raises_value_error() -> None:
    """Setting an invalid level should raise ValueError."""
    logger = FemtoLogger("core")
    with pytest.raises(ValueError, match="level"):
        logger.set_level("INVALID")


def test_log_fast_path_without_exc_or_stack() -> None:
    """Without exc_info or stack_info, output should be simple."""
    logger = FemtoLogger("core")
    output = logger.log("INFO", "simple")
    assert output == "core [INFO] simple", (
        f"fast path produced unexpected output: {output!r}"
    )
