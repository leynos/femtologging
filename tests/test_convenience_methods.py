"""Tests for convenience logging methods, ``isEnabledFor``, and ``getLogger``.

Purpose
-------
Verify that the stdlib-compatible convenience API on ``FemtoLogger``
behaves correctly: ``debug``, ``info``, ``warning``, ``error``,
``critical``, ``exception``, ``isEnabledFor``, and the module-level
``getLogger`` alias.

Notes
-----
These tests exercise the Python-facing signatures exposed by PyO3.
Each convenience method delegates to the internal ``log()`` machinery,
so the tests focus on level tagging, filtering, ``exc_info`` /
``stack_info`` passthrough, and the ``exception()`` auto-capture
semantics rather than duplicating full logging-pipeline coverage.

Because ``sys.exc_info()`` is frame-scoped, the ``active_exception``
fixture (a ``@contextmanager`` generator) cannot propagate the active
exception into the ``with``-body's frame.  Tests that need to verify
traceback content use an inline ``try/except`` instead.

Examples
--------
>>> from femtologging import FemtoLogger, getLogger
>>> logger = getLogger("example")
>>> logger.set_level("DEBUG")
>>> logger.isEnabledFor("INFO")
True
>>> logger.info("hello") is not None
True

"""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import (
    FemtoLogger,
    get_logger,
    getLogger,
)
from tests.logger_support import assert_output_contains

# -- getLogger alias ----------------------------------------------------------


def _raise_for_capture(error: Exception) -> typ.NoReturn:
    """Raise ``error`` from a nested frame so the caller can catch it.

    Several tests need a live ``sys.exc_info()`` inside their own frame.
    Raising here rather than inline in the ``try`` block keeps the raise out
    of the block that handles it.
    """
    raise error


# -- getLogger alias ----------------------------------------------------------


def test_get_logger_alias_returns_same_instance() -> None:
    """``getLogger`` and ``get_logger`` must return the same logger."""
    a = get_logger("alias.test")
    b = getLogger("alias.test")
    assert a is b, "getLogger and get_logger should return the same instance"


def test_get_logger_alias_is_callable() -> None:
    """``getLogger`` must be directly callable."""
    logger = getLogger("alias.callable")
    assert isinstance(logger, FemtoLogger), "getLogger should return a FemtoLogger"


# -- isEnabledFor -------------------------------------------------------------


@pytest.mark.parametrize(
    ("threshold", "query", "expected"),
    [
        pytest.param("WARNING", "WARNING", True, id="own-level-is-enabled"),
        pytest.param("INFO", "ERROR", True, id="level-above-threshold-is-enabled"),
        pytest.param("ERROR", "DEBUG", False, id="debug-below-error-is-disabled"),
        pytest.param("ERROR", "INFO", False, id="info-below-error-is-disabled"),
        pytest.param("ERROR", "WARN", False, id="warn-below-error-is-disabled"),
    ],
)
def test_is_enabled_for_respects_threshold(
    threshold: str, query: str, *, expected: bool
) -> None:
    """``isEnabledFor`` reports enablement relative to the logger's level."""
    logger = FemtoLogger("enabled.threshold")
    logger.set_level(threshold)
    assert logger.isEnabledFor(query) == expected, (
        f"with level {threshold}, isEnabledFor({query}) should be {expected}"
    )


def test_is_enabled_for_all_level_boundaries() -> None:
    """Exhaustively check each level boundary."""
    logger = FemtoLogger("enabled.all")
    levels = ("TRACE", "DEBUG", "INFO", "WARN", "ERROR", "CRITICAL")
    for i, threshold in enumerate(levels):
        logger.set_level(threshold)
        for j, query in enumerate(levels):
            expected = j >= i
            assert logger.isEnabledFor(query) == expected, (
                f"set_level({threshold}), isEnabledFor({query}) expected {expected}"
            )


def test_is_enabled_for_invalid_level_raises() -> None:
    """Invalid level strings should raise ValueError."""
    logger = FemtoLogger("enabled.invalid")
    with pytest.raises(ValueError, match="level"):
        logger.isEnabledFor("BOGUS")


# -- Convenience methods (debug/info/warning/error/critical) ------------------


@pytest.mark.parametrize(
    ("method", "expected_level"),
    [
        ("debug", "DEBUG"),
        ("info", "INFO"),
        ("warning", "WARN"),
        ("error", "ERROR"),
        ("critical", "CRITICAL"),
    ],
)
def test_convenience_method_formats_correctly(method: str, expected_level: str) -> None:
    """Each convenience method should produce the correct level tag."""
    logger = FemtoLogger("conv")
    logger.set_level("TRACE")
    result = getattr(logger, method)("hello")
    assert result == f"conv [{expected_level}] hello", (
        f"{method}() should format as 'conv [{expected_level}] hello', got {result!r}"
    )


@pytest.mark.parametrize(
    ("method", "level_threshold", "should_emit"),
    [
        ("debug", "INFO", False),
        ("debug", "DEBUG", True),
        ("info", "WARN", False),
        ("info", "INFO", True),
        ("warning", "ERROR", False),
        ("warning", "WARN", True),
        ("error", "CRITICAL", False),
        ("error", "ERROR", True),
        ("critical", "CRITICAL", True),
    ],
)
def test_convenience_methods_respect_level(
    method: str, level_threshold: str, *, should_emit: bool
) -> None:
    """Convenience methods must honour the logger's level threshold."""
    logger = FemtoLogger("filter")
    logger.set_level(level_threshold)
    result = getattr(logger, method)("test")
    if should_emit:
        assert result is not None, (
            f"{method}() should emit at threshold {level_threshold}"
        )
    else:
        assert result is None, (
            f"{method}() should be filtered at threshold {level_threshold}"
        )


def test_convenience_method_with_exc_info() -> None:
    """Convenience methods should accept and propagate exc_info.

    Uses an inline ``try/except`` because ``sys.exc_info()`` is
    frame-scoped and the ``active_exception`` fixture (a generator-based
    context manager) runs in a separate frame.
    """
    logger = FemtoLogger("exc")
    logger.set_level("TRACE")
    try:
        _raise_for_capture(TypeError("boom"))
    except TypeError:
        output = logger.error("caught", exc_info=True)
    assert_output_contains(
        output, "TypeError", "Traceback", context="error(exc_info=True)"
    )


def test_convenience_method_with_stack_info() -> None:
    """Convenience methods should accept stack_info."""
    logger = FemtoLogger("stack")
    logger.set_level("TRACE")
    output = logger.info("check", stack_info=True)
    assert_output_contains(
        output, "Stack (most recent call last)", context="info(stack_info=True)"
    )


def test_exception_captures_active_exception() -> None:
    """``exception()`` should produce output with an active exception context."""
    logger = FemtoLogger("exc.auto")
    try:
        _raise_for_capture(ValueError("auto capture"))
    except ValueError:
        output = logger.exception("caught")
    assert_output_contains(
        output,
        "[ERROR]",
        "ValueError",
        "Traceback",
        context="exception() auto-capture",
    )


def test_exception_auto_capture_traceback() -> None:
    """``exception()`` captures traceback when ``sys.exc_info()`` is populated.

    Uses an inline ``try/except`` because ``sys.exc_info()`` is
    frame-scoped and the ``active_exception`` fixture (a generator-based
    context manager) runs in a separate frame.
    """
    logger = FemtoLogger("exc.tb")
    try:
        _raise_for_capture(RuntimeError("traceback check"))
    except RuntimeError:
        output = logger.exception("caught")
    assert_output_contains(
        output,
        "RuntimeError",
        "traceback check",
        "Traceback",
        context="exception() traceback capture",
    )


def test_exception_logs_at_error_level() -> None:
    """``exception()`` should log at ERROR level."""
    logger = FemtoLogger("exc.level")
    logger.set_level("ERROR")
    try:
        _raise_for_capture(ValueError("level check"))
    except ValueError:
        output = logger.exception("caught")
    assert_output_contains(output, "[ERROR]", context="exception() at ERROR threshold")


def test_exception_filtered_below_error() -> None:
    """``exception()`` should be filtered when level > ERROR."""
    logger = FemtoLogger("exc.filter")
    logger.set_level("CRITICAL")
    try:
        _raise_for_capture(ValueError("filtered"))
    except ValueError:
        output = logger.exception("caught")
    assert output is None, "exception() should be filtered when level is CRITICAL"


def test_exception_with_no_active_exception() -> None:
    """``exception()`` with no active exception logs plain message."""
    logger = FemtoLogger("exc.none")
    # ruff: ignore[log-exception-outside-except-handler] the point of the test
    # is exception() called with no live sys.exc_info().
    output = logger.exception("no error active")
    assert output == "exc.none [ERROR] no error active", (
        f"exception() without an active exception should log the bare message, "
        f"got {output!r}"
    )


@pytest.mark.parametrize(
    ("logger_name", "exc_info"),
    [
        pytest.param("exc.false", False, id="explicit-false"),
        pytest.param("exc.explicit_none", None, id="explicit-none"),
    ],
)
def test_exception_with_falsy_exc_info_suppresses_capture(
    logger_name: str, *, exc_info: bool | None
) -> None:
    """A falsy explicit ``exc_info`` suppresses capture (stdlib semantics).

    The Python wrapper distinguishes omitted ``exc_info`` from an explicit
    falsy value using a sentinel, matching ``logging.Logger.exception()``.
    """
    logger = FemtoLogger(logger_name)
    try:
        _raise_for_capture(ValueError("suppressed"))
    except ValueError:
        output = logger.exception("caught", exc_info=exc_info)
    assert output == f"{logger_name} [ERROR] caught", (
        f"exc_info={exc_info!r} should suppress traceback capture, got {output!r}"
    )


def test_exception_with_exc_info_instance() -> None:
    """``exception()`` with an exception instance should capture it."""
    logger = FemtoLogger("exc.inst")
    # ruff: ignore[log-exception-outside-except-handler] the instance is passed
    # explicitly, so no active handler is required.
    output = logger.exception("caught", exc_info=KeyError("specific"))
    assert_output_contains(output, "KeyError", context="exception(exc_info=<instance>)")


def test_error_with_exc_info_captures_traceback() -> None:
    """``error(exc_info=True)`` captures traceback in an active handler.

    Uses an inline ``try/except`` because ``sys.exc_info()`` is
    frame-scoped and cannot be propagated via a generator-based context
    manager.
    """
    logger = FemtoLogger("exc.inline")
    logger.set_level("TRACE")
    try:
        _raise_for_capture(ValueError("boom"))
    except ValueError:
        output = logger.error("caught", exc_info=True)
    assert_output_contains(
        output,
        "ValueError",
        "Traceback",
        context="error(exc_info=True) inside handler",
    )
