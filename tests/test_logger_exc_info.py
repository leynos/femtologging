"""Exception and stack capture tests for :class:`FemtoLogger`.

Covers the ``exc_info`` and ``stack_info`` arguments of
``FemtoLogger.log``: how each accepted form is rendered into the
formatted output, and which forms are rejected.
"""

from __future__ import annotations

import sys
import types
import typing as typ
import warnings

import pytest

from femtologging import FemtoLogger
from tests.logger_support import assert_output_contains, raise_exception


def test_log_with_exc_info_true_captures_exception() -> None:
    """exc_info=True should capture the current exception."""
    logger = FemtoLogger("core")
    try:
        raise_exception()
    except ValueError:
        output = logger.log("ERROR", "caught", exc_info=True)

    assert_output_contains(output, "ValueError", "Traceback")


def test_log_with_exc_info_instance() -> None:
    """exc_info with an exception instance should capture it."""
    logger = FemtoLogger("core")
    exc = KeyError("missing")
    output = logger.log("ERROR", "caught", exc_info=exc)

    assert_output_contains(output, "KeyError")


def test_log_with_exc_info_tuple() -> None:
    """exc_info as a (type, value, traceback) tuple should capture that traceback."""
    logger = FemtoLogger("core")

    try:
        raise_exception(KeyError, "missing")
    except KeyError as exc:
        exc_info = (KeyError, exc, exc.__traceback__)

    output = logger.log("ERROR", "caught", exc_info=exc_info)

    assert_output_contains(output, "KeyError", "missing", "Traceback")


def test_log_with_exc_info_tuple_preserves_explicit_traceback() -> None:
    """Explicit traceback in tuple persists when __traceback__ is None."""
    logger = FemtoLogger("core")

    try:
        raise_exception(KeyError, "missing")
    except KeyError as exc:
        # Capture the traceback before clearing it
        tb = exc.__traceback__
        exc_info = (KeyError, exc, tb)
        # Clear the exception's __traceback__ attribute
        exc.__traceback__ = None

    output = logger.log("ERROR", "caught", exc_info=exc_info)

    # The output should still contain the traceback (and at least one stack
    # frame) because it was passed explicitly in the tuple.
    assert_output_contains(output, "KeyError", "Traceback", "raise_exception")


@pytest.mark.parametrize(
    "bad_exc_info",
    ["bad", 123],
    ids=["string", "integer"],
)
def test_log_with_invalid_exc_info_type(bad_exc_info: object) -> None:
    """Invalid exc_info type should raise a TypeError with a useful message."""
    logger = FemtoLogger("core")

    with pytest.raises(TypeError, match="exc_info"):
        logger.log("ERROR", "bad exc_info", exc_info=typ.cast("typ.Any", bad_exc_info))


@pytest.mark.parametrize(
    "exc_info",
    [True, False, None],
    ids=["true_without_active_exception", "false", "none"],
)
def test_log_without_active_exception_omits_traceback(*, exc_info: bool | None) -> None:
    """Falsy exc_info, or exc_info=True with no active exception, adds nothing."""
    logger = FemtoLogger("core")
    output = logger.log("INFO", "no error", exc_info=exc_info)
    assert output == "core [INFO] no error", (
        f"exc_info={exc_info!r} unexpectedly altered output: {output!r}"
    )


def test_log_with_stack_info_true() -> None:
    """stack_info=True should include call stack."""
    logger = FemtoLogger("core")
    output = logger.log("INFO", "debug", stack_info=True)

    assert_output_contains(output, "Stack (most recent call last)")


def test_log_with_both_exc_and_stack_info() -> None:
    """Both exc_info and stack_info should work together."""
    logger = FemtoLogger("core")
    try:
        raise_exception(RuntimeError)
    except RuntimeError:
        output = logger.log("ERROR", "debug", exc_info=True, stack_info=True)

    assert_output_contains(output, "Stack (most recent call last)", "RuntimeError")


def test_exc_info_no_deprecation_warning(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Exception capture must not trigger exc_type DeprecationWarning.

    Python 3.13 deprecated ``TracebackException.exc_type`` in favour of
    ``exc_type_qualname`` / ``exc_type_module``. Verify that our capture
    path avoids the deprecated attribute. We record all warnings and then
    assert none match the specific ``exc_type`` deprecation, so unrelated
    DeprecationWarnings from third-party code cannot cause false failures.
    """
    logger = FemtoLogger("core")

    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always", DeprecationWarning)

        # exc_info=True with an active exception
        try:
            raise_exception(ValueError, "deprecation check")
        except ValueError:
            output = logger.log("ERROR", "caught", exc_info=True)
        assert_output_contains(output, "ValueError")

        # exc_info with an exception instance directly
        output = logger.log("ERROR", "caught", exc_info=RuntimeError("instance check"))
        assert_output_contains(output, "RuntimeError")

        # exc_info with a 3-tuple
        try:
            raise_exception(KeyError, "tuple check")
        except KeyError as e:
            exc_info = (KeyError, e, e.__traceback__)
        output = logger.log("ERROR", "caught", exc_info=exc_info)
        assert_output_contains(output, "KeyError")

        # exc_info with a custom exception from a non-builtin module
        mod = types.ModuleType("custom_mod")
        monkeypatch.setitem(sys.modules, "custom_mod", mod)
        custom_cls = type("CustomError", (Exception,), {"__module__": "custom_mod"})
        typ.cast("typ.Any", mod).CustomError = custom_cls
        output = logger.log("ERROR", "caught", exc_info=custom_cls("module check"))
        assert_output_contains(output, "custom_mod.CustomError")

    exc_type_warnings = [w for w in caught if "exc_type" in str(w.message)]
    assert not exc_type_warnings, (
        f"exc_type deprecation warnings emitted: {exc_type_warnings}"
    )
