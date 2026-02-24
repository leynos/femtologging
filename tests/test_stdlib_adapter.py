"""Tests for StdlibHandlerAdapter bridging stdlib handlers to femtologging."""

from __future__ import annotations

import io
import logging
import typing as typ

import pytest

from femtologging import FemtoLogger, StdlibHandlerAdapter


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
) -> str:
    """Configure, log a single message, and return captured output.

    Parameters
    ----------
    stream
        The StringIO backing the stdlib handler.
    stdlib_handler
        The stdlib StreamHandler to configure.
    adapter
        The StdlibHandlerAdapter wrapping *stdlib_handler*.
    logger_name
        Name for the FemtoLogger.
    level
        Femtologging level string (e.g. ``"INFO"``).
    message
        Log message text.
    formatter
        Optional stdlib Formatter to apply to *stdlib_handler*.
    logger_level
        Optional femtologging level to set on the logger.
    handler_level
        Optional stdlib numeric level to set on *stdlib_handler*.
    stack_info
        If ``True``, capture the current call stack.

    Returns
    -------
    str
        The captured output from *stream*.

    """
    if formatter is not None:
        stdlib_handler.setFormatter(formatter)
    if handler_level is not None:
        stdlib_handler.setLevel(handler_level)

    logger = FemtoLogger(logger_name)
    if logger_level is not None:
        logger.set_level(logger_level)
    logger.add_handler(adapter)
    logger.log(level, message, stack_info=stack_info)
    del logger

    return stream.getvalue()


# -- Tests ----------------------------------------------------------------


class TestStdlibHandlerAdapterConstruction:
    """Verify adapter construction validates the wrapped handler."""

    @staticmethod
    def test_wraps_stdlib_handler() -> None:
        """Adapter should accept a logging.Handler instance."""
        handler = logging.StreamHandler(io.StringIO())
        adapter = StdlibHandlerAdapter(handler)
        assert adapter._handler is handler

    @staticmethod
    def test_rejects_non_handler() -> None:
        """Adapter should raise TypeError for non-Handler objects."""
        with pytest.raises(TypeError, match=r"expected a logging\.Handler"):
            StdlibHandlerAdapter("not a handler")  # type: ignore[arg-type]

    @staticmethod
    def test_rejects_plain_object() -> None:
        """Adapter should raise TypeError for arbitrary objects."""
        with pytest.raises(TypeError, match=r"expected a logging\.Handler"):
            StdlibHandlerAdapter(object())  # type: ignore[arg-type]


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
    def test_error_level_mapped(
        stream: io.StringIO,
        stdlib_handler: logging.StreamHandler[io.StringIO],
        adapter: StdlibHandlerAdapter,
    ) -> None:
        """ERROR level should be mapped to stdlib ERROR."""
        output = log_and_capture(
            stream,
            stdlib_handler,
            adapter,
            logger_name="app",
            level="ERROR",
            message="failure",
            formatter=logging.Formatter("%(levelname)s"),
        )
        assert "ERROR" in output

    @staticmethod
    def test_debug_level_mapped(
        stream: io.StringIO,
        stdlib_handler: logging.StreamHandler[io.StringIO],
        adapter: StdlibHandlerAdapter,
    ) -> None:
        """DEBUG level should be mapped to stdlib DEBUG."""
        output = log_and_capture(
            stream,
            stdlib_handler,
            adapter,
            logger_name="app",
            level="DEBUG",
            message="trace detail",
            formatter=logging.Formatter("%(levelname)s %(message)s"),
            logger_level="DEBUG",
            handler_level=logging.DEBUG,
        )
        assert "DEBUG" in output
        assert "trace detail" in output

    @staticmethod
    def test_critical_level_mapped(
        stream: io.StringIO,
        stdlib_handler: logging.StreamHandler[io.StringIO],
        adapter: StdlibHandlerAdapter,
    ) -> None:
        """CRITICAL level should be mapped to stdlib CRITICAL."""
        output = log_and_capture(
            stream,
            stdlib_handler,
            adapter,
            logger_name="app",
            level="CRITICAL",
            message="fatal",
            formatter=logging.Formatter("%(levelname)s"),
        )
        assert "CRITICAL" in output

    @staticmethod
    def test_warn_level_mapped(
        stream: io.StringIO,
        stdlib_handler: logging.StreamHandler[io.StringIO],
        adapter: StdlibHandlerAdapter,
    ) -> None:
        """WARN level should be mapped to stdlib WARNING."""
        output = log_and_capture(
            stream,
            stdlib_handler,
            adapter,
            logger_name="app",
            level="WARN",
            message="caution",
            formatter=logging.Formatter("%(levelname)s"),
        )
        assert "WARNING" in output


class TestExceptionForwarding:
    """Verify that exception information reaches the stdlib handler."""

    @staticmethod
    def test_exc_info_forwarded_as_text(
        stream: io.StringIO,
        stdlib_handler: logging.StreamHandler[io.StringIO],
        adapter: StdlibHandlerAdapter,
    ) -> None:
        """Exception payload should appear as exc_text on the LogRecord."""
        stdlib_handler.setFormatter(logging.Formatter("%(message)s\n%(exc_text)s"))

        logger = FemtoLogger("app")
        logger.add_handler(adapter)

        try:
            _raise_value_error()
        except ValueError:
            logger.log("ERROR", "caught", exc_info=True)

        del logger

        output = stream.getvalue()
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
    def test_flush_delegates() -> None:
        """flush() should call the wrapped handler's flush()."""
        calls: list[str] = []

        class SpyHandler(logging.Handler):
            @typ.override
            def emit(self, record: logging.LogRecord) -> None:
                pass

            @typ.override
            def flush(self) -> None:
                calls.append("flush")

        adapter = StdlibHandlerAdapter(SpyHandler())
        adapter.flush()
        assert calls == ["flush"]

    @staticmethod
    def test_close_delegates() -> None:
        """close() should call the wrapped handler's close()."""
        calls: list[str] = []

        class SpyHandler(logging.Handler):
            @typ.override
            def emit(self, record: logging.LogRecord) -> None:
                pass

            @typ.override
            def close(self) -> None:
                calls.append("close")
                super().close()

        adapter = StdlibHandlerAdapter(SpyHandler())
        adapter.close()
        assert calls == ["close"]


class TestHandleFallback:
    """Verify the static handle method exists for validation."""

    @staticmethod
    def test_handle_is_callable() -> None:
        """The handle fallback must be callable for add_handler validation."""
        adapter = StdlibHandlerAdapter(logging.StreamHandler(io.StringIO()))
        assert callable(adapter.handle)

    @staticmethod
    def test_handle_returns_none() -> None:
        """The handle fallback should be a no-op returning None."""
        adapter = StdlibHandlerAdapter(logging.StreamHandler(io.StringIO()))
        result = adapter.handle("logger", "INFO", "msg")
        assert result is None


class TestLogRecordAttributes:
    """Verify LogRecord attributes are populated from the femtologging record."""

    @staticmethod
    def test_logger_name_on_record() -> None:
        """The LogRecord name should match the femtologging logger name."""
        emitted: list[logging.LogRecord] = []

        class CapturingHandler(logging.Handler):
            @typ.override
            def emit(self, record: logging.LogRecord) -> None:
                emitted.append(record)

        adapter = StdlibHandlerAdapter(CapturingHandler())

        logger = FemtoLogger("myapp.sub")
        logger.add_handler(adapter)
        logger.log("INFO", "test")
        del logger

        assert len(emitted) == 1
        assert emitted[0].name == "myapp.sub"
        assert emitted[0].levelno == logging.INFO
        assert emitted[0].levelname == "INFO"
        assert emitted[0].getMessage() == "test"

    @staticmethod
    def test_timestamp_populated() -> None:
        """The LogRecord created field should reflect the record timestamp."""
        emitted: list[logging.LogRecord] = []

        class CapturingHandler(logging.Handler):
            @typ.override
            def emit(self, record: logging.LogRecord) -> None:
                emitted.append(record)

        adapter = StdlibHandlerAdapter(CapturingHandler())

        logger = FemtoLogger("app")
        logger.add_handler(adapter)
        logger.log("INFO", "stamped")
        del logger

        assert len(emitted) == 1
        # Timestamp should be a positive float (UNIX epoch seconds).
        assert isinstance(emitted[0].created, float)
        assert emitted[0].created > 0


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
