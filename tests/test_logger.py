"""Tests for :class:`FemtoLogger`."""

from __future__ import annotations

import collections.abc as cabc
import typing as typ
from pathlib import Path

import pytest

from femtologging import FemtoFileHandler, FemtoLogger

FileHandlerFactory = cabc.Callable[
    [Path, int, int], typ.ContextManager[FemtoFileHandler]
]


@pytest.mark.parametrize(
    ("name", "level", "message", "expected"),
    [
        ("core", "INFO", "hello", "core [INFO] hello"),
        ("sys", "ERROR", "fail", "sys [ERROR] fail"),
        ("", "INFO", "empty name", " [INFO] empty name"),
        ("core", "INFO", "", "core [INFO] "),
        ("i18n", "INFO", "こんにちは世界", "i18n [INFO] こんにちは世界"),
        (
            "n" * 1000,
            "INFO",
            "m" * 1000,
            f"{'n' * 1000} [INFO] {'m' * 1000}",
        ),
    ],
)
def test_log_formats_message(
    name: str, level: str, message: str, expected: str
) -> None:
    """Logger.log should format messages using configured formatter."""
    logger = FemtoLogger(name)
    assert logger.log(level, message) == expected


def test_log_respects_logger_level() -> None:
    """Messages below the logger level should be ignored."""
    logger = FemtoLogger("core")
    logger.set_level("ERROR")
    assert logger.log("INFO", "ignored") is None
    assert logger.log("ERROR", "processed") == "core [ERROR] processed"


def test_level_parsing_and_filtering() -> None:
    """Verify log level parsing and filtering across variants."""
    logger = FemtoLogger("core")
    for lvl in ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "CRITICAL"]:
        logger.set_level(lvl)
        assert logger.log(lvl, "ok") is not None

    logger.set_level("ERROR")
    assert logger.log("WARN", "drop") is None
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
    assert path1.read_text() == "core [INFO] hello\n"
    assert path2.read_text() == "core [INFO] hello\n"


class CollectingHandler:
    """Simple handler used to verify Python handler support."""

    def __init__(self) -> None:
        """Initialize an empty record buffer."""
        self.records: list[tuple[str, str, str]] = []

    def handle(self, logger: str, level: str, message: str) -> None:
        """Collect handled records for later assertions."""
        self.records.append((logger, level, message))


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


def test_python_handler_invocation() -> None:
    """Python handlers should receive records via PyHandler."""
    logger = FemtoLogger("core")
    collector = CollectingHandler()
    logger.add_handler(collector)
    logger.log("INFO", "ok")
    del logger
    assert collector.records == [("core", "INFO", "ok")]


def test_python_handler_invocation_multiple_messages() -> None:
    """Python handlers should receive every emitted record."""
    logger = FemtoLogger("core")
    collector = CollectingHandler()
    logger.add_handler(collector)
    logger.log("INFO", "first")
    logger.log("ERROR", "second")
    del logger
    assert collector.records == [
        ("core", "INFO", "first"),
        ("core", "ERROR", "second"),
    ]


def test_level_getter_returns_current_level() -> None:
    """Level property should reflect changes from set_level."""
    logger = FemtoLogger("core")
    for lvl in ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "CRITICAL"]:
        logger.set_level(lvl)
        assert logger.level == lvl


def test_level_default_is_info() -> None:
    """New loggers should default to INFO level."""
    logger = FemtoLogger("core")
    assert logger.level == "INFO"


def test_set_level_invalid_raises_value_error() -> None:
    """Setting an invalid level should raise ValueError."""
    logger = FemtoLogger("core")
    with pytest.raises(ValueError, match="level"):
        logger.set_level("INVALID")


def _raise_exception(exc_type: type[BaseException] = ValueError, msg: str = "") -> None:
    """Raise an exception of the given type for testing.

    Parameters
    ----------
    exc_type
        The exception class to raise. Defaults to ValueError.
    msg
        The exception message. Defaults to empty string.

    """
    if msg:
        raise exc_type(msg)
    raise exc_type()


def test_log_with_exc_info_true_captures_exception() -> None:
    """exc_info=True should capture the current exception."""
    logger = FemtoLogger("core")
    try:
        _raise_exception()
    except ValueError:
        output = logger.log("ERROR", "caught", exc_info=True)

    assert output is not None
    assert "ValueError" in output
    assert "Traceback" in output


def test_log_with_exc_info_true_no_exception() -> None:
    """exc_info=True with no active exception should not add traceback."""
    logger = FemtoLogger("core")
    output = logger.log("INFO", "no error", exc_info=True)
    assert output == "core [INFO] no error"


def test_log_with_exc_info_instance() -> None:
    """exc_info with an exception instance should capture it."""
    logger = FemtoLogger("core")
    exc = KeyError("missing")
    output = logger.log("ERROR", "caught", exc_info=exc)

    assert output is not None
    assert "KeyError" in output


def test_log_with_exc_info_tuple() -> None:
    """exc_info as a (type, value, traceback) tuple should capture that traceback."""
    logger = FemtoLogger("core")

    try:
        _raise_exception(KeyError, "missing")
    except KeyError as exc:
        exc_info = (KeyError, exc, exc.__traceback__)

    output = logger.log("ERROR", "caught", exc_info=exc_info)

    assert output is not None
    assert "KeyError" in output
    assert "missing" in output
    assert "Traceback" in output


def test_log_with_exc_info_tuple_preserves_explicit_traceback() -> None:
    """Explicit traceback in tuple persists when __traceback__ is None."""
    logger = FemtoLogger("core")

    try:
        _raise_exception(KeyError, "missing")
    except KeyError as exc:
        # Capture the traceback before clearing it
        tb = exc.__traceback__
        exc_info = (KeyError, exc, tb)
        # Clear the exception's __traceback__ attribute
        exc.__traceback__ = None

    output = logger.log("ERROR", "caught", exc_info=exc_info)

    # The output should still contain the traceback because we passed it
    # explicitly in the tuple
    assert output is not None
    assert "KeyError" in output
    assert "Traceback" in output
    # Should contain at least one stack frame
    assert "_raise_exception" in output


def test_log_with_invalid_exc_info_type() -> None:
    """Invalid exc_info type should raise a TypeError with a useful message."""
    logger = FemtoLogger("core")

    with pytest.raises(TypeError, match="exc_info"):
        logger.log("ERROR", "bad exc_info", exc_info="bad")  # type: ignore[arg-type]

    with pytest.raises(TypeError, match="exc_info"):
        logger.log("ERROR", "bad exc_info", exc_info=123)  # type: ignore[arg-type]


def test_log_with_exc_info_false_and_none() -> None:
    """exc_info=False and exc_info=None should behave as no traceback capture."""
    logger = FemtoLogger("core")

    output_false = logger.log("INFO", "no error", exc_info=False)
    assert output_false == "core [INFO] no error"

    output_none = logger.log("INFO", "no error", exc_info=None)
    assert output_none == "core [INFO] no error"


def test_log_with_stack_info_true() -> None:
    """stack_info=True should include call stack."""
    logger = FemtoLogger("core")
    output = logger.log("INFO", "debug", stack_info=True)

    assert output is not None
    assert "Stack (most recent call last)" in output


def test_log_with_both_exc_and_stack_info() -> None:
    """Both exc_info and stack_info should work together."""
    logger = FemtoLogger("core")
    try:
        _raise_exception(RuntimeError)
    except RuntimeError:
        output = logger.log("ERROR", "debug", exc_info=True, stack_info=True)

    assert output is not None
    assert "Stack (most recent call last)" in output
    assert "RuntimeError" in output


def test_log_fast_path_without_exc_or_stack() -> None:
    """Without exc_info or stack_info, output should be simple."""
    logger = FemtoLogger("core")
    output = logger.log("INFO", "simple")
    assert output == "core [INFO] simple"


class RecordCollectingHandler:
    """Handler that uses handle_record for structured access."""

    def __init__(self) -> None:
        """Initialize an empty record buffer."""
        self.records: list[dict[str, typ.Any]] = []

    @staticmethod
    def handle(_logger: str, _level: str, _message: str) -> None:
        """Fallback handle method (required by FemtoLogger validation)."""
        # Should not be called when handle_record is present
        return

    def handle_record(self, record: dict[str, typ.Any]) -> None:
        """Collect full records for later assertions."""
        self.records.append(record)


def test_handle_record_receives_structured_payload() -> None:
    """Handlers with handle_record should receive the full record dict."""
    logger = FemtoLogger("core")
    handler = RecordCollectingHandler()
    logger.add_handler(handler)

    sentinel_msg = "sentinel message"
    try:
        _raise_exception(ValueError, sentinel_msg)
    except ValueError:
        logger.log("ERROR", "caught", exc_info=True)

    del logger

    assert len(handler.records) == 1
    record = handler.records[0]
    assert record["logger"] == "core"
    assert record["level"] == "ERROR"
    assert record["message"] == "caught"

    # Verify exc_info structure
    assert "exc_info" in record
    exc_info = record["exc_info"]

    # Basic shape
    assert exc_info["type_name"] == "ValueError"

    # schema_version should be present and numeric
    assert "schema_version" in exc_info
    assert isinstance(exc_info["schema_version"], int)

    # message should match the original exception message
    assert exc_info["message"] == sentinel_msg

    # frames should be a non-empty list with expected structure
    assert "frames" in exc_info
    assert isinstance(exc_info["frames"], list)
    assert len(exc_info["frames"]) > 0

    # First frame should have expected keys
    frame = exc_info["frames"][0]
    assert "filename" in frame
    assert "lineno" in frame
    assert "function" in frame


def test_handle_record_fallback_to_handle() -> None:
    """Handlers without handle_record should use the 3-arg handle method."""
    logger = FemtoLogger("core")
    handler = CollectingHandler()
    logger.add_handler(handler)
    logger.log("INFO", "test")
    del logger
    assert handler.records == [("core", "INFO", "test")]


def test_handle_record_includes_stack_info() -> None:
    """handle_record should include stack_info when present."""
    logger = FemtoLogger("core")
    handler = RecordCollectingHandler()
    logger.add_handler(handler)

    logger.log("INFO", "debug", stack_info=True)

    del logger

    assert len(handler.records) == 1
    record = handler.records[0]
    assert "stack_info" in record
    assert "frames" in record["stack_info"]
    assert len(record["stack_info"]["frames"]) > 0


def test_handle_record_includes_both_exc_and_stack_info() -> None:
    """handle_record should include both exc_info and stack_info when present."""
    logger = FemtoLogger("core")
    handler = RecordCollectingHandler()
    logger.add_handler(handler)

    try:
        _raise_exception(ValueError, "test error")
    except ValueError:
        logger.log("ERROR", "caught", exc_info=True, stack_info=True)

    del logger

    assert len(handler.records) == 1
    record = handler.records[0]

    # Verify exc_info is present
    assert "exc_info" in record
    assert record["exc_info"]["type_name"] == "ValueError"
    assert record["exc_info"]["message"] == "test error"
    assert "frames" in record["exc_info"]
    assert len(record["exc_info"]["frames"]) > 0

    # Verify stack_info is present
    assert "stack_info" in record
    assert "frames" in record["stack_info"]
    assert len(record["stack_info"]["frames"]) > 0


class MutableHandler:
    """Handler whose capabilities can be mutated after construction."""

    def __init__(self) -> None:
        """Initialize an empty record buffer for both dispatch paths."""
        self.handle_calls: list[tuple[str, str, str]] = []
        self.handle_record_calls: list[dict[str, typ.Any]] = []

    def handle(self, logger: str, level: str, message: str) -> None:
        """Legacy 3-argument handle method."""
        self.handle_calls.append((logger, level, message))


def test_handler_gains_handle_record_after_registration() -> None:
    """Adding handle_record after registration should not change dispatch.

    Capability detection happens once at registration time. If a handler
    is registered without handle_record and later gains one, the legacy
    handle() method should still be called because the cached capability
    is frozen at registration.
    """
    logger = FemtoLogger("core")
    handler = MutableHandler()

    # Register without handle_record
    logger.add_handler(handler)

    # Dynamically add handle_record after registration
    def late_handle_record(record: dict[str, typ.Any]) -> None:
        handler.handle_record_calls.append(record)

    handler.handle_record = late_handle_record  # type: ignore[attr-defined]

    # Log a message
    logger.log("INFO", "test message")
    del logger

    # handle() should be called (frozen capability)
    assert handler.handle_calls == [("core", "INFO", "test message")]
    # handle_record() should NOT be called
    assert handler.handle_record_calls == []


def test_handler_dispatch_path_frozen_at_registration() -> None:
    """Dispatch path (handle vs handle_record) is frozen at registration.

    The capability check that determines whether to use handle_record or
    handle is performed once at registration time. If a handler is
    registered with handle_record present, the handle_record dispatch path
    will be used for all subsequent log records, even if the method is
    later replaced.

    Note: Deleting handle_record after registration would cause an
    AttributeError because the cached capability tells the runtime to call
    a now-missing method. This test demonstrates the frozen dispatch path
    by replacing the method and verifying handle() is not called.
    """
    logger = FemtoLogger("core")
    handler = MutableHandler()

    # Add handle_record before registration
    def initial_handle_record(record: dict[str, typ.Any]) -> None:
        handler.handle_record_calls.append(record)

    handler.handle_record = initial_handle_record  # type: ignore[attr-defined]

    # Register with handle_record present - this freezes the dispatch path
    logger.add_handler(handler)

    # Replace handle_record with a different implementation after registration
    def replacement_handle_record(record: dict[str, typ.Any]) -> None:
        handler.handle_record_calls.append(record)

    handler.handle_record = replacement_handle_record  # type: ignore[attr-defined]

    # Log a message
    logger.log("INFO", "test message")
    del logger

    # handle_record dispatch path was frozen at registration time
    assert len(handler.handle_record_calls) == 1
    assert handler.handle_record_calls[0]["message"] == "test message"
    # handle() should NOT be called because dispatch path was frozen to handle_record
    assert handler.handle_calls == []


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
    import sys
    import types
    import warnings

    logger = FemtoLogger("core")

    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always", DeprecationWarning)

        # exc_info=True with an active exception
        try:
            _raise_exception(ValueError, "deprecation check")
        except ValueError:
            output = logger.log("ERROR", "caught", exc_info=True)
        assert output is not None, "exc_info=True should produce output"
        assert "ValueError" in output, f"expected 'ValueError' in output: {output}"

        # exc_info with an exception instance directly
        exc = RuntimeError("instance check")
        output = logger.log("ERROR", "caught", exc_info=exc)
        assert output is not None, "exc_info with instance should produce output"
        assert "RuntimeError" in output, f"expected 'RuntimeError' in output: {output}"

        # exc_info with a 3-tuple
        try:
            _raise_exception(KeyError, "tuple check")
        except KeyError as e:
            exc_info = (KeyError, e, e.__traceback__)
        output = logger.log("ERROR", "caught", exc_info=exc_info)
        assert output is not None, "exc_info with 3-tuple should produce output"
        assert "KeyError" in output, f"expected 'KeyError' in output: {output}"

        # exc_info with a custom exception from a non-builtin module
        mod = types.ModuleType("custom_mod")
        monkeypatch.setitem(sys.modules, "custom_mod", mod)
        custom_cls = type("CustomError", (Exception,), {"__module__": "custom_mod"})
        mod.CustomError = custom_cls  # type: ignore[attr-defined]
        exc = custom_cls("module check")
        output = logger.log("ERROR", "caught", exc_info=exc)
        assert output is not None, "exc_info with custom module should produce output"
        assert "custom_mod.CustomError" in output, (
            f"expected 'custom_mod.CustomError' in output: {output}"
        )

    exc_type_warnings = [w for w in caught if "exc_type" in str(w.message)]
    assert exc_type_warnings == [], (
        f"exc_type deprecation warnings emitted: {exc_type_warnings}"
    )
