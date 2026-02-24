"""Tests for StdlibHandlerAdapter bridging stdlib handlers to femtologging."""

from __future__ import annotations

import dataclasses
import io
import logging
import typing as typ

import pytest

from femtologging import FemtoLogger, StdlibHandlerAdapter
from femtologging.adapter import (
    TRACE_LEVEL_NUM,
    FemtoRecord,
    _ensure_trace_level,
    _make_log_record,
    _stdlib_levelno,
)


@dataclasses.dataclass(frozen=True, slots=True)
class _LevelCase:
    """A single level-mapping test case."""

    level: str
    expected_level: str
    message: str
    logger_level: str | None = None
    handler_level: int | None = None


def _raise_value_error(msg: str = "test error") -> None:
    """Raise a ValueError for testing exc_info capture."""
    raise ValueError(msg)


# -- Fixtures -------------------------------------------------------------


@pytest.fixture
def stream() -> io.StringIO:
    """Return a fresh StringIO for capturing handler output."""
    return io.StringIO()


@pytest.fixture
def stdlib_handler(stream: io.StringIO) -> logging.StreamHandler[io.StringIO]:
    """Return a StreamHandler writing to *stream*."""
    return logging.StreamHandler(stream)


@pytest.fixture
def adapter(
    stdlib_handler: logging.StreamHandler[io.StringIO],
) -> StdlibHandlerAdapter:
    """Return a StdlibHandlerAdapter wrapping *stdlib_handler*."""
    return StdlibHandlerAdapter(stdlib_handler)


# -- Helpers --------------------------------------------------------------


def log_and_capture(
    stream: io.StringIO,
    stdlib_handler: logging.StreamHandler[io.StringIO],
    adapter: StdlibHandlerAdapter,
    /,
    *,
    logger_name: str,
    level: str,
    message: str,
    formatter: logging.Formatter | None = None,
    logger_level: str | None = None,
    handler_level: int | None = None,
    stack_info: bool = False,
    exc_info: bool = False,
) -> str:
    """Configure, log a single message via *adapter*, and return captured output."""
    if formatter is not None:
        stdlib_handler.setFormatter(formatter)
    if handler_level is not None:
        stdlib_handler.setLevel(handler_level)

    logger = FemtoLogger(logger_name)
    if logger_level is not None:
        logger.set_level(logger_level)
    logger.add_handler(adapter)
    logger.log(level, message, stack_info=stack_info, exc_info=exc_info)
    del logger

    return stream.getvalue()


# -- Tests ----------------------------------------------------------------


class TestStdlibHandlerAdapterConstruction:
    """Verify adapter construction validates the wrapped handler."""

    @staticmethod
    def test_wraps_stdlib_handler() -> None:
        """Adapter should forward records to the wrapped stdlib handler."""
        stream = io.StringIO()
        handler = logging.StreamHandler(stream)
        handler.setFormatter(logging.Formatter("%(message)s"))
        adapter = StdlibHandlerAdapter(handler)

        adapter.handle_record(
            {"logger": "test", "level": "INFO", "message": "wrapped-ok"},
        )
        handler.flush()

        output = stream.getvalue()
        assert "wrapped-ok" in output, (
            f"wrapped handler did not receive the record: {output!r}"
        )

    @staticmethod
    def test_trace_level_registered_on_init() -> None:
        """TRACE level should be registered with stdlib logging after adapter init."""
        _ensure_trace_level()
        assert logging.getLevelName(TRACE_LEVEL_NUM) == "TRACE", (
            f"TRACE level not registered: "
            f"getLevelName({TRACE_LEVEL_NUM}) = "
            f"{logging.getLevelName(TRACE_LEVEL_NUM)!r}"
        )

    @staticmethod
    def test_rejects_non_handler() -> None:
        """Adapter should raise TypeError for non-Handler objects."""
        with pytest.raises(TypeError, match=r"expected a logging\.Handler"):
            StdlibHandlerAdapter(typ.cast("logging.Handler", "not a handler"))

    @staticmethod
    def test_rejects_plain_object() -> None:
        """Adapter should raise TypeError for arbitrary objects."""
        with pytest.raises(TypeError, match=r"expected a logging\.Handler"):
            StdlibHandlerAdapter(typ.cast("logging.Handler", object()))


class TestHandleRecordDispatch:
    """Verify that femtologging records are translated and emitted."""

    @staticmethod
    def test_basic_message_emitted(
        stream: io.StringIO,
        stdlib_handler: logging.StreamHandler[io.StringIO],
        adapter: StdlibHandlerAdapter,
    ) -> None:
        """A simple log message should appear in the wrapped handler's output."""
        output = log_and_capture(
            stream,
            stdlib_handler,
            adapter,
            logger_name="myapp",
            level="INFO",
            message="hello world",
            formatter=logging.Formatter("%(name)s %(levelname)s %(message)s"),
        )
        assert "myapp" in output
        assert "INFO" in output
        assert "hello world" in output

    @staticmethod
    @pytest.mark.parametrize(
        "case",
        [
            _LevelCase("ERROR", "ERROR", "failure"),
            _LevelCase("DEBUG", "DEBUG", "trace detail", "DEBUG", logging.DEBUG),
            _LevelCase("CRITICAL", "CRITICAL", "fatal"),
            _LevelCase("WARN", "WARNING", "caution"),
            _LevelCase("TRACE", "TRACE", "lowest", "TRACE", TRACE_LEVEL_NUM),
        ],
        ids=["error", "debug", "critical", "warn", "trace"],
    )
    def test_level_mapped(
        stream: io.StringIO,
        stdlib_handler: logging.StreamHandler[io.StringIO],
        adapter: StdlibHandlerAdapter,
        case: _LevelCase,
    ) -> None:
        """Each femtologging level should map to its stdlib equivalent."""
        output = log_and_capture(
            stream,
            stdlib_handler,
            adapter,
            logger_name="app",
            level=case.level,
            message=case.message,
            formatter=logging.Formatter("%(levelname)s %(message)s"),
            logger_level=case.logger_level,
            handler_level=case.handler_level,
        )
        assert case.expected_level in output, (
            f"expected level {case.expected_level!r} not found in output "
            f"for logger='app', level={case.level!r}, message={case.message!r}: "
            f"{output!r}"
        )
        assert case.message in output, (
            f"expected message {case.message!r} not found in output "
            f"for logger='app', level={case.level!r}: {output!r}"
        )


class TestExceptionForwarding:
    """Verify that exception information reaches the stdlib handler."""

    @staticmethod
    def test_exc_info_forwarded_as_text(
        stream: io.StringIO,
        stdlib_handler: logging.StreamHandler[io.StringIO],
        adapter: StdlibHandlerAdapter,
    ) -> None:
        """Exception payload should appear as exc_text on the LogRecord."""
        try:
            _raise_value_error()
        except ValueError:
            output = log_and_capture(
                stream,
                stdlib_handler,
                adapter,
                logger_name="app",
                level="ERROR",
                message="caught",
                formatter=logging.Formatter("%(message)s\n%(exc_text)s"),
                exc_info=True,
            )
        assert "caught" in output
        assert "ValueError" in output

    @staticmethod
    def test_stack_info_forwarded(
        stream: io.StringIO,
        stdlib_handler: logging.StreamHandler[io.StringIO],
        adapter: StdlibHandlerAdapter,
    ) -> None:
        """Stack trace payload should appear as stack_info on the LogRecord."""
        output = log_and_capture(
            stream,
            stdlib_handler,
            adapter,
            logger_name="app",
            level="INFO",
            message="trace",
            formatter=logging.Formatter("%(message)s"),
            stack_info=True,
        )
        assert "trace" in output
        # stdlib Formatter appends stack_info after the message
        assert "Stack (most recent call last)" in output


class TestDelegation:
    """Verify flush() and close() delegate to the wrapped handler."""

    @staticmethod
    @pytest.mark.parametrize(
        "method_name",
        ["flush", "close"],
        ids=["flush", "close"],
    )
    def test_delegation(method_name: str) -> None:
        """flush() and close() should delegate to the wrapped handler."""
        calls: list[str] = []

        class SpyHandler(logging.Handler):
            @typ.override
            def emit(self, record: logging.LogRecord) -> None:
                pass

            @typ.override
            def flush(self) -> None:
                calls.append("flush")

            @typ.override
            def close(self) -> None:
                calls.append("close")
                super().close()

        adapter = StdlibHandlerAdapter(SpyHandler())
        getattr(adapter, method_name)()
        assert calls == [method_name]


class TestHandleFallback:
    """Verify the static handle method exists for validation."""

    @staticmethod
    def test_handle_is_callable() -> None:
        """The handle fallback must be callable for add_handler validation."""
        adapter = StdlibHandlerAdapter(logging.StreamHandler(io.StringIO()))
        assert callable(adapter.handle)

    @staticmethod
    def test_handle_emits_warning() -> None:
        """Calling handle() directly should emit a RuntimeWarning."""
        adapter = StdlibHandlerAdapter(logging.StreamHandler(io.StringIO()))
        with pytest.warns(RuntimeWarning, match=r"handle_record\(\) should be used"):
            adapter.handle("logger", "INFO", "msg")


class TestLogRecordAttributes:
    """Verify LogRecord attributes are populated from the femtologging record."""

    @staticmethod
    @pytest.mark.parametrize(
        ("logger_name", "level", "message", "check_attrs"),
        [
            (
                "myapp.sub",
                "INFO",
                "test",
                {
                    "name": "myapp.sub",
                    "levelno": logging.INFO,
                    "levelname": "INFO",
                    "message": "test",
                },
            ),
            (
                "app",
                "INFO",
                "stamped",
                {"created_type": float, "created_positive": True},
            ),
        ],
        ids=["logger_name", "timestamp"],
    )
    def test_record_attributes(
        logger_name: str,
        level: str,
        message: str,
        check_attrs: dict[str, object],
    ) -> None:
        """LogRecord attributes should be populated from the femtologging record."""
        emitted: list[logging.LogRecord] = []

        class CapturingHandler(logging.Handler):
            @typ.override
            def emit(self, record: logging.LogRecord) -> None:
                emitted.append(record)

        adapter = StdlibHandlerAdapter(CapturingHandler())

        logger = FemtoLogger(logger_name)
        logger.add_handler(adapter)
        logger.log(level, message)
        del logger

        assert len(emitted) == 1
        record = emitted[0]

        ctx = f"logger={logger_name!r}, level={level!r}, message={message!r}"
        if "name" in check_attrs:
            assert record.name == check_attrs["name"], (
                f"record.name mismatch ({ctx}): "
                f"got {record.name!r}, expected {check_attrs['name']!r}"
            )
        if "levelno" in check_attrs:
            assert record.levelno == check_attrs["levelno"], (
                f"record.levelno mismatch ({ctx}): "
                f"got {record.levelno!r}, expected {check_attrs['levelno']!r}"
            )
        if "levelname" in check_attrs:
            assert record.levelname == check_attrs["levelname"], (
                f"record.levelname mismatch ({ctx}): "
                f"got {record.levelname!r}, expected {check_attrs['levelname']!r}"
            )
        if "message" in check_attrs:
            assert record.getMessage() == check_attrs["message"], (
                f"record message mismatch ({ctx}): "
                f"got {record.getMessage()!r}, expected {check_attrs['message']!r}"
            )
        if "created_type" in check_attrs:
            expected_type = check_attrs["created_type"]
            assert isinstance(expected_type, type), (
                f"created_type check_attr must be a type ({ctx})"
            )
            assert isinstance(record.created, expected_type), (
                f"record.created type mismatch ({ctx}): "
                f"got {type(record.created).__name__}, "
                f"expected {expected_type.__name__}"
            )
        if "created_positive" in check_attrs:
            assert record.created > 0, (
                f"record.created not positive ({ctx}): got {record.created}"
            )

    @staticmethod
    def test_msecs_consistent_with_created() -> None:
        """Milliseconds should be derived from created when timestamp is overridden."""
        timestamp = 1700000000.456
        record = _make_log_record(
            {"metadata": {"timestamp": timestamp}},
        )
        assert record.created == timestamp
        expected_msecs = (timestamp - int(timestamp)) * 1000.0
        assert record.msecs == pytest.approx(expected_msecs)

    @staticmethod
    def test_relative_created_consistent_with_created() -> None:
        """Relative-created should be recomputed when timestamp is overridden."""
        timestamp = 1700000000.456
        record = _make_log_record(
            {"metadata": {"timestamp": timestamp}},
        )
        start_time: float = getattr(logging, "_startTime", 0.0)
        expected = (timestamp - start_time) * 1000.0
        assert record.relativeCreated == pytest.approx(expected), (
            f"relativeCreated mismatch: got {record.relativeCreated}, "
            f"expected {expected}"
        )


class TestLevelFallback:
    """Verify _stdlib_levelno falls back to WARNING for unknown levels."""

    @staticmethod
    @pytest.mark.parametrize(
        ("record", "expected"),
        [
            ({"levelno": 99}, logging.WARNING),
            ({"level": "FOO"}, logging.WARNING),
            ({}, logging.WARNING),
        ],
        ids=["unknown_levelno", "unknown_name", "empty_record"],
    )
    def test_unknown_level_falls_back(
        record: FemtoRecord,
        expected: int,
    ) -> None:
        """Unknown or missing level information should fall back to WARNING."""
        assert _stdlib_levelno(record) == expected


class TestPublicExport:
    """Verify the adapter is accessible from the top-level package."""

    @staticmethod
    def test_importable_from_package() -> None:
        """StdlibHandlerAdapter should be importable from femtologging."""
        import femtologging

        assert hasattr(femtologging, "StdlibHandlerAdapter")
        assert femtologging.StdlibHandlerAdapter is StdlibHandlerAdapter

    @staticmethod
    def test_in_all() -> None:
        """StdlibHandlerAdapter should be listed in __all__."""
        import femtologging

        assert "StdlibHandlerAdapter" in femtologging.__all__
