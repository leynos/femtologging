"""LogRecord population and delegation tests for :class:`StdlibHandlerAdapter`.

Covers the ``logging.LogRecord`` attributes the adapter populates, the
level fallback for unknown levels, delegation of ``flush``/``close`` to the
wrapped handler, the ``handle`` compatibility shim, and package exports.
Construction and dispatch live in ``tests/test_stdlib_adapter.py``.
"""

from __future__ import annotations

import io
import logging
import time
import typing as typ

import pytest

import femtologging
from femtologging import FemtoLogger, StdlibHandlerAdapter

if typ.TYPE_CHECKING:
    from femtologging.adapter import FemtoRecord

OVERRIDDEN_TIMESTAMP = 1700000000.456


class CapturingHandler(logging.Handler):
    """Handler that captures emitted LogRecords for test inspection."""

    def __init__(self) -> None:
        """Initialize with an empty records list."""
        super().__init__()
        self.records: list[logging.LogRecord] = []

    @typ.override
    def emit(self, record: logging.LogRecord) -> None:
        self.records.append(record)


def _sole_emitted_record(capturing: CapturingHandler) -> logging.LogRecord:
    """Return the single record captured by *capturing*.

    Returns
    -------
    logging.LogRecord
        The only record the handler received.

    Examples
    --------
    >>> _sole_emitted_record(capturing).levelname
    'INFO'

    """
    assert len(capturing.records) == 1, (
        f"expected 1 emitted record, got {len(capturing.records)}"
    )
    return capturing.records[0]


def _emit_femto_record(record: FemtoRecord) -> logging.LogRecord:
    """Push *record* through an adapter and return the resulting LogRecord.

    Returns
    -------
    logging.LogRecord
        The record the wrapped handler received.

    Examples
    --------
    >>> _emit_femto_record({"level": "INFO"}).levelname
    'INFO'

    """
    capturing = CapturingHandler()
    StdlibHandlerAdapter(capturing).handle_record(record)
    return _sole_emitted_record(capturing)


def _emit_via_logger(logger_name: str, level: str, message: str) -> logging.LogRecord:
    """Log through a FemtoLogger and return the adapted LogRecord.

    Returns
    -------
    logging.LogRecord
        The record the wrapped handler received.

    Examples
    --------
    >>> _emit_via_logger("app", "INFO", "hi").getMessage()
    'hi'

    """
    capturing = CapturingHandler()
    logger = FemtoLogger(logger_name)
    logger.add_handler(StdlibHandlerAdapter(capturing))
    logger.log(level, message)
    del logger
    return _sole_emitted_record(capturing)


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
        assert calls == [method_name], f"expected [{method_name!r}], got {calls!r}"


class TestHandleFallback:
    """Verify the static handle method exists for validation."""

    @staticmethod
    def test_handle_is_callable() -> None:
        """The handle fallback must be callable for add_handler validation."""
        adapter = StdlibHandlerAdapter(logging.StreamHandler(io.StringIO()))
        assert callable(adapter.handle), "adapter.handle should be callable"

    @staticmethod
    def test_handle_emits_warning() -> None:
        """Calling handle() directly should emit a RuntimeWarning."""
        adapter = StdlibHandlerAdapter(logging.StreamHandler(io.StringIO()))
        with pytest.warns(RuntimeWarning, match=r"handle_record\(\) should be used"):
            adapter.handle("logger", "INFO", "msg")


class TestLogRecordAttributes:
    """Verify LogRecord attributes are populated from the femtologging record."""

    @staticmethod
    def test_identity_attributes_copied() -> None:
        """Name, level, and message should mirror the femtologging record."""
        record = _emit_via_logger("myapp.sub", "INFO", "test")
        assert record.name == "myapp.sub", f"record.name mismatch: {record.name!r}"
        assert record.levelno == logging.INFO, (
            f"record.levelno mismatch: {record.levelno!r}"
        )
        assert record.levelname == "INFO", (
            f"record.levelname mismatch: {record.levelname!r}"
        )
        assert record.getMessage() == "test", (
            f"record message mismatch: {record.getMessage()!r}"
        )

    @staticmethod
    def test_default_timestamp_is_populated() -> None:
        """Records without an explicit timestamp still get a wall-clock time."""
        record = _emit_via_logger("app", "INFO", "stamped")
        assert isinstance(record.created, float), (
            f"record.created must be a float, got {type(record.created).__name__}"
        )
        assert record.created > 0, f"record.created not positive: got {record.created}"

    @staticmethod
    def test_msecs_consistent_with_created() -> None:
        """Milliseconds should be derived from created when timestamp is overridden."""
        record = _emit_femto_record({"metadata": {"timestamp": OVERRIDDEN_TIMESTAMP}})
        assert record.created == OVERRIDDEN_TIMESTAMP, (
            f"record.created mismatch: got {record.created}, "
            f"expected {OVERRIDDEN_TIMESTAMP}"
        )
        expected_msecs = (OVERRIDDEN_TIMESTAMP - int(OVERRIDDEN_TIMESTAMP)) * 1000.0
        assert record.msecs == pytest.approx(expected_msecs), (
            f"record.msecs mismatch: got {record.msecs}, expected {expected_msecs}"
        )

    @staticmethod
    def test_asctime_uses_overridden_timestamp() -> None:
        """Formatted asctime should reflect the overridden timestamp."""
        # logging.Formatter uses local time unless its converter is overridden.
        record = _emit_femto_record({"metadata": {"timestamp": OVERRIDDEN_TIMESTAMP}})
        formatted = logging.Formatter("%(asctime)s").format(record)

        expected_prefix = time.strftime(
            "%Y-%m-%d %H:%M:%S",
            time.localtime(OVERRIDDEN_TIMESTAMP),
        )
        assert formatted.startswith(expected_prefix), (
            f"asctime does not start with expected prefix: "
            f"got {formatted!r}, expected prefix {expected_prefix!r}"
        )
        # The default asctime format appends ",456" for milliseconds
        assert ",456" in formatted, (
            f"asctime missing expected milliseconds ',456': {formatted!r}"
        )

    @staticmethod
    def test_relative_created_consistent_with_created() -> None:
        """Relative-created should be recomputed when timestamp is overridden."""
        record = _emit_femto_record({"metadata": {"timestamp": OVERRIDDEN_TIMESTAMP}})
        start_time: float = getattr(logging, "_startTime", time.time())
        expected = (record.created - start_time) * 1000.0
        assert record.relativeCreated == pytest.approx(expected), (
            f"relativeCreated mismatch: got {record.relativeCreated}, "
            f"expected {expected}"
        )


class TestLevelFallback:
    """Verify the adapter falls back to WARNING for unknown levels."""

    @staticmethod
    @pytest.mark.parametrize(
        "femto_record",
        [{"levelno": 99}, {"level": "FOO"}, {}],
        ids=["unknown_levelno", "unknown_name", "empty_record"],
    )
    def test_unknown_level_falls_back(femto_record: FemtoRecord) -> None:
        """Unknown or missing level information should fall back to WARNING."""
        actual = _emit_femto_record(femto_record).levelno
        assert actual == logging.WARNING, (
            f"expected levelno {logging.WARNING} for record {femto_record!r}, "
            f"got {actual}"
        )


class TestPublicExport:
    """Verify the adapter is accessible from the top-level package."""

    @staticmethod
    def test_importable_from_package() -> None:
        """StdlibHandlerAdapter should be importable from femtologging."""
        assert hasattr(femtologging, "StdlibHandlerAdapter"), (
            "StdlibHandlerAdapter not found on femtologging module"
        )
        assert femtologging.StdlibHandlerAdapter is StdlibHandlerAdapter, (
            "femtologging.StdlibHandlerAdapter is not the expected class"
        )

    @staticmethod
    def test_in_all() -> None:
        """StdlibHandlerAdapter should be listed in __all__."""
        assert "StdlibHandlerAdapter" in femtologging.__all__, (
            f"StdlibHandlerAdapter not in __all__: {femtologging.__all__!r}"
        )
