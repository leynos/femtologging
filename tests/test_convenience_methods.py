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

if typ.TYPE_CHECKING:
    import collections.abc as cabc

import pytest

from femtologging import FemtoLogger, get_logger, getLogger

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


def test_is_enabled_for_at_same_level() -> None:
    """Logger should report enabled for its own level."""
    logger = FemtoLogger("enabled.same")
    logger.set_level("WARNING")
    assert logger.isEnabledFor("WARNING"), "should be enabled at own level"


def test_is_enabled_for_above_level() -> None:
    """Logger should report enabled for levels above its threshold."""
    logger = FemtoLogger("enabled.above")
    logger.set_level("INFO")
    assert logger.isEnabledFor("ERROR"), "should be enabled for levels above threshold"


def test_is_enabled_for_below_level() -> None:
    """Logger should report disabled for levels below its threshold."""
    logger = FemtoLogger("enabled.below")
    logger.set_level("ERROR")
    assert not logger.isEnabledFor("DEBUG"), (
        "DEBUG should be disabled when level is ERROR"
    )
    assert not logger.isEnabledFor("INFO"), (
        "INFO should be disabled when level is ERROR"
    )
    assert not logger.isEnabledFor("WARN"), (
        "WARN should be disabled when level is ERROR"
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
    msg = "boom"
    try:
        raise TypeError(msg)  # noqa: TRY301  # deliberate re-raise to populate sys.exc_info
    except TypeError:
        output = logger.error("caught", exc_info=True)
    assert output is not None, "error(exc_info=True) should produce output"
    assert "TypeError" in output, "output should contain the exception type"
    assert "Traceback" in output, "output should contain traceback text"


def test_convenience_method_with_stack_info() -> None:
    """Convenience methods should accept stack_info."""
    logger = FemtoLogger("stack")
    logger.set_level("TRACE")
    output = logger.info("check", stack_info=True)
    assert output is not None, "info(stack_info=True) should produce output"
    assert "Stack (most recent call last)" in output, (
        "output should contain stack trace"
    )


# -- exception() --------------------------------------------------------------


def test_exception_captures_active_exception(
    active_exception: cabc.Callable[[str], typ.ContextManager[None]],
) -> None:
    """``exception()`` should produce output with an active exception context."""
    logger = FemtoLogger("exc.auto")
    with active_exception("auto capture"):
        output = logger.exception("caught")  # noqa: LOG004  # TODO(#340): inside fixture-based handler
    assert output is not None, "exception() should produce output"
    assert "[ERROR]" in output, "output should contain [ERROR] level tag"


def test_exception_auto_capture_traceback() -> None:
    """``exception()`` captures traceback when ``sys.exc_info()`` is populated.

    Uses an inline ``try/except`` because ``sys.exc_info()`` is
    frame-scoped and the ``active_exception`` fixture (a generator-based
    context manager) runs in a separate frame.
    """
    logger = FemtoLogger("exc.tb")
    msg = "traceback check"
    try:
        raise RuntimeError(msg)  # noqa: TRY301  # TODO(#340): deliberate re-raise to populate sys.exc_info
    except RuntimeError:
        output = logger.exception("caught")
    assert output is not None, "exception() should produce output"
    assert "RuntimeError" in output, "output should contain RuntimeError"
    assert "traceback check" in output, "output should contain exception message"
    assert "Traceback" in output, "output should contain traceback text"


def test_exception_logs_at_error_level(
    active_exception: cabc.Callable[[str], typ.ContextManager[None]],
) -> None:
    """``exception()`` should log at ERROR level."""
    logger = FemtoLogger("exc.level")
    logger.set_level("ERROR")
    with active_exception("level check"):
        output = logger.exception("caught")  # noqa: LOG004  # TODO(#340): inside fixture-based handler
    assert output is not None, "exception() should produce output at ERROR level"
    assert "[ERROR]" in output, "output should contain [ERROR] level tag"


def test_exception_filtered_below_error(
    active_exception: cabc.Callable[[str], typ.ContextManager[None]],
) -> None:
    """``exception()`` should be filtered when level > ERROR."""
    logger = FemtoLogger("exc.filter")
    logger.set_level("CRITICAL")
    with active_exception("filtered"):
        output = logger.exception("caught")  # noqa: LOG004  # TODO(#340): inside fixture-based handler
    assert output is None, "exception() should be filtered when level is CRITICAL"


def test_exception_with_no_active_exception() -> None:
    """``exception()`` with no active exception logs plain message."""
    logger = FemtoLogger("exc.none")
    output = logger.exception("no error active")  # noqa: LOG004  # TODO(#340): testing exception() outside handler
    assert output is not None, (
        "exception() should produce output even without active exception"
    )
    assert output == "exc.none [ERROR] no error active", (
        f"expected plain message, got {output!r}"
    )


def test_exception_with_explicit_exc_info_false(
    active_exception: cabc.Callable[[str], typ.ContextManager[None]],
) -> None:
    """``exception()`` with exc_info=False should not capture."""
    logger = FemtoLogger("exc.false")
    with active_exception("suppressed"):
        output = logger.exception("caught", exc_info=False)  # noqa: LOG004, LOG007  # TODO(#340): testing explicit False override
    assert output == "exc.false [ERROR] caught", (
        f"exc_info=False should suppress capture, got {output!r}"
    )


def test_exception_with_explicit_exc_info_none_suppresses(
    active_exception: cabc.Callable[[str], typ.ContextManager[None]],
) -> None:
    """``exception(exc_info=None)`` suppresses capture (stdlib semantics).

    The Python wrapper distinguishes omitted ``exc_info`` from explicit
    ``None`` using a sentinel.  Explicit ``None`` is falsy, so capture
    is suppressed — matching ``logging.Logger.exception()`` behaviour.
    """
    logger = FemtoLogger("exc.explicit_none")
    with active_exception("suppressed"):
        output = logger.exception("caught", exc_info=None)  # noqa: LOG004, LOG007  # TODO(#340): testing exc_info=None
    assert output == "exc.explicit_none [ERROR] caught", (
        f"exc_info=None should suppress capture, got {output!r}"
    )


def test_exception_with_exc_info_instance() -> None:
    """``exception()`` with an exception instance should capture it."""
    logger = FemtoLogger("exc.inst")
    exc = KeyError("specific")
    output = logger.exception("caught", exc_info=exc)  # noqa: LOG004  # TODO(#340): testing exception() outside handler
    assert output is not None, "exception(exc_info=<instance>) should produce output"
    assert "KeyError" in output, "output should contain the exception type"


def test_error_with_exc_info_captures_traceback() -> None:
    """``error(exc_info=True)`` captures traceback in an active handler.

    Uses an inline ``try/except`` because ``sys.exc_info()`` is
    frame-scoped and cannot be propagated via a generator-based context
    manager.
    """
    logger = FemtoLogger("exc.inline")
    logger.set_level("TRACE")
    msg = "boom"
    try:
        raise ValueError(msg)  # noqa: TRY301  # TODO(#340): deliberate re-raise to populate sys.exc_info
    except ValueError:
        output = logger.error("caught", exc_info=True)
    assert output is not None, "error(exc_info=True) should produce output"
    assert "ValueError" in output, "output should contain the exception type"
    assert "Traceback" in output, "output should contain traceback text"
