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

import pytest

from femtologging import FemtoLogger, get_logger, getLogger

# -- getLogger alias ----------------------------------------------------------


def test_get_logger_alias_returns_same_instance() -> None:
    """``getLogger`` and ``get_logger`` must return the same logger."""
    a = get_logger("alias.test")
    b = getLogger("alias.test")
    assert a is b


def test_get_logger_alias_is_callable() -> None:
    """``getLogger`` must be directly callable."""
    logger = getLogger("alias.callable")
    assert isinstance(logger, FemtoLogger)


# -- isEnabledFor -------------------------------------------------------------


def test_is_enabled_for_at_same_level() -> None:
    """Logger should report enabled for its own level."""
    logger = FemtoLogger("enabled.same")
    logger.set_level("WARNING")
    assert logger.isEnabledFor("WARNING")


def test_is_enabled_for_above_level() -> None:
    """Logger should report enabled for levels above its threshold."""
    logger = FemtoLogger("enabled.above")
    logger.set_level("INFO")
    assert logger.isEnabledFor("ERROR")


def test_is_enabled_for_below_level() -> None:
    """Logger should report disabled for levels below its threshold."""
    logger = FemtoLogger("enabled.below")
    logger.set_level("ERROR")
    assert not logger.isEnabledFor("DEBUG")
    assert not logger.isEnabledFor("INFO")
    assert not logger.isEnabledFor("WARN")


def test_is_enabled_for_all_level_boundaries() -> None:
    """Exhaustively check each level boundary."""
    logger = FemtoLogger("enabled.all")
    levels = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "CRITICAL"]
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
        assert result is not None
    else:
        assert result is None


def test_convenience_method_with_exc_info() -> None:
    """Convenience methods should accept exc_info."""
    logger = FemtoLogger("exc")
    logger.set_level("TRACE")
    msg = "boom"
    try:
        raise ValueError(msg)  # noqa: TRY301 — deliberate re-raise to populate sys.exc_info
    except ValueError:
        output = logger.error("caught", exc_info=True)
    assert output is not None
    assert "ValueError" in output
    assert "Traceback" in output


def test_convenience_method_with_stack_info() -> None:
    """Convenience methods should accept stack_info."""
    logger = FemtoLogger("stack")
    logger.set_level("TRACE")
    output = logger.info("check", stack_info=True)
    assert output is not None
    assert "Stack (most recent call last)" in output


# -- exception() --------------------------------------------------------------


def test_exception_captures_active_exception() -> None:
    """``exception()`` should automatically capture exc_info."""
    logger = FemtoLogger("exc.auto")
    msg = "auto capture"
    try:
        raise RuntimeError(msg)  # noqa: TRY301 — deliberate re-raise to populate sys.exc_info
    except RuntimeError:
        output = logger.exception("caught")
    assert output is not None
    assert "RuntimeError" in output
    assert "auto capture" in output
    assert "Traceback" in output


def test_exception_logs_at_error_level() -> None:
    """``exception()`` should log at ERROR level."""
    logger = FemtoLogger("exc.level")
    logger.set_level("ERROR")
    msg = "level check"
    try:
        raise ValueError(msg)  # noqa: TRY301 — deliberate re-raise to populate sys.exc_info
    except ValueError:
        output = logger.exception("caught")
    assert output is not None
    assert "[ERROR]" in output


def test_exception_filtered_below_error() -> None:
    """``exception()`` should be filtered when level > ERROR."""
    logger = FemtoLogger("exc.filter")
    logger.set_level("CRITICAL")
    msg = "filtered"
    try:
        raise ValueError(msg)  # noqa: TRY301 — deliberate re-raise to populate sys.exc_info
    except ValueError:
        output = logger.exception("caught")
    assert output is None


def test_exception_with_no_active_exception() -> None:
    """``exception()`` with no active exception logs plain message."""
    logger = FemtoLogger("exc.none")
    output = logger.exception("no error active")  # noqa: LOG004 — testing exception() outside handler
    assert output is not None
    assert output == "exc.none [ERROR] no error active"


def test_exception_with_explicit_exc_info_false() -> None:
    """``exception()`` with exc_info=False should not capture."""
    logger = FemtoLogger("exc.false")
    msg = "suppressed"
    try:
        raise ValueError(msg)  # noqa: TRY301 — deliberate re-raise to populate sys.exc_info
    except ValueError:
        output = logger.exception("caught", exc_info=False)  # noqa: LOG007 — testing explicit False override
    assert output == "exc.false [ERROR] caught"


def test_exception_with_explicit_exc_info_none() -> None:
    """``exception(exc_info=None)`` should suppress capture like stdlib."""
    logger = FemtoLogger("exc.explicit_none")
    msg = "should not capture"
    try:
        raise ValueError(msg)  # noqa: TRY301 — deliberate re-raise to populate sys.exc_info
    except ValueError:
        output = logger.exception("caught", exc_info=None)  # noqa: LOG007 — testing explicit None override
    assert output == "exc.explicit_none [ERROR] caught"


def test_exception_with_exc_info_instance() -> None:
    """``exception()`` with an exception instance should capture it."""
    logger = FemtoLogger("exc.inst")
    exc = KeyError("specific")
    output = logger.exception("caught", exc_info=exc)  # noqa: LOG004 — testing exception() outside handler
    assert output is not None
    assert "KeyError" in output
