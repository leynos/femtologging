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
    def test_basic_message_emitted() -> None:
        """A simple log message should appear in the wrapped handler's output."""
        stream = io.StringIO()
        stdlib_handler = logging.StreamHandler(stream)
        stdlib_handler.setFormatter(
            logging.Formatter("%(name)s %(levelname)s %(message)s")
        )
        adapter = StdlibHandlerAdapter(stdlib_handler)

        logger = FemtoLogger("myapp")
        logger.add_handler(adapter)
        logger.log("INFO", "hello world")
        del logger

        output = stream.getvalue()
        assert "myapp" in output
        assert "INFO" in output
        assert "hello world" in output

    @staticmethod
    def test_error_level_mapped() -> None:
        """ERROR level should be mapped to stdlib ERROR."""
        stream = io.StringIO()
        stdlib_handler = logging.StreamHandler(stream)
        stdlib_handler.setFormatter(logging.Formatter("%(levelname)s"))
        adapter = StdlibHandlerAdapter(stdlib_handler)

        logger = FemtoLogger("app")
        logger.add_handler(adapter)
        logger.log("ERROR", "failure")
        del logger

        output = stream.getvalue()
        assert "ERROR" in output

    @staticmethod
    def test_debug_level_mapped() -> None:
        """DEBUG level should be mapped to stdlib DEBUG."""
        stream = io.StringIO()
        stdlib_handler = logging.StreamHandler(stream)
        stdlib_handler.setLevel(logging.DEBUG)
        stdlib_handler.setFormatter(logging.Formatter("%(levelname)s %(message)s"))
        adapter = StdlibHandlerAdapter(stdlib_handler)

        logger = FemtoLogger("app")
        logger.set_level("DEBUG")
        logger.add_handler(adapter)
        logger.log("DEBUG", "trace detail")
        del logger

        output = stream.getvalue()
        assert "DEBUG" in output
        assert "trace detail" in output

    @staticmethod
    def test_critical_level_mapped() -> None:
        """CRITICAL level should be mapped to stdlib CRITICAL."""
        stream = io.StringIO()
        stdlib_handler = logging.StreamHandler(stream)
        stdlib_handler.setFormatter(logging.Formatter("%(levelname)s"))
        adapter = StdlibHandlerAdapter(stdlib_handler)

        logger = FemtoLogger("app")
        logger.add_handler(adapter)
        logger.log("CRITICAL", "fatal")
        del logger

        output = stream.getvalue()
        assert "CRITICAL" in output

    @staticmethod
    def test_warn_level_mapped() -> None:
        """WARN level should be mapped to stdlib WARNING."""
        stream = io.StringIO()
        stdlib_handler = logging.StreamHandler(stream)
        stdlib_handler.setFormatter(logging.Formatter("%(levelname)s"))
        adapter = StdlibHandlerAdapter(stdlib_handler)

        logger = FemtoLogger("app")
        logger.add_handler(adapter)
        logger.log("WARN", "caution")
        del logger

        output = stream.getvalue()
        assert "WARNING" in output


class TestExceptionForwarding:
    """Verify that exception information reaches the stdlib handler."""

    @staticmethod
    def test_exc_info_forwarded_as_text() -> None:
        """Exception payload should appear as exc_text on the LogRecord."""
        stream = io.StringIO()
        stdlib_handler = logging.StreamHandler(stream)
        stdlib_handler.setFormatter(logging.Formatter("%(message)s\n%(exc_text)s"))
        adapter = StdlibHandlerAdapter(stdlib_handler)

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
    def test_stack_info_forwarded() -> None:
        """Stack trace payload should appear as stack_info on the LogRecord."""
        stream = io.StringIO()
        stdlib_handler = logging.StreamHandler(stream)
        stdlib_handler.setFormatter(logging.Formatter("%(message)s"))
        adapter = StdlibHandlerAdapter(stdlib_handler)

        logger = FemtoLogger("app")
        logger.add_handler(adapter)
        logger.log("INFO", "trace", stack_info=True)
        del logger

        output = stream.getvalue()
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
