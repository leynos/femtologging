"""Handler-level threshold tests for :class:`StdlibHandlerAdapter`.

Covers acceptance criteria 1-4 of issue #435: the adapter discards a record
whose mapped stdlib level falls below the wrapped handler's configured
``level``, preserves ``NOTSET`` semantics, and still runs the handler's
filter-and-lock path exactly once for accepted records. Dynamic ``setLevel()``
changes are covered in ``tests/test_stdlib_adapter_records.py``; construction
and dispatch live in ``tests/test_stdlib_adapter.py``.
"""

from __future__ import annotations

import logging
from unittest import mock

import pytest

from femtologging import StdlibHandlerAdapter
from tests.stdlib_adapter_support import (
    AdapterProbe,
    CapturingHandler,
    CountingFilter,
    HandlerLevelCase,
    HandlerSetup,
    LogRequest,
    dispatch,
    emitted_messages,
)


class TestHandlerLevelThreshold:
    """Verify the wrapped handler's level gates dispatch."""

    @staticmethod
    @pytest.mark.parametrize(
        "case",
        [
            HandlerLevelCase("TRACE", "lowest", None),
            HandlerLevelCase("DEBUG", "detail", None),
            HandlerLevelCase("INFO", "note", None),
            HandlerLevelCase("WARNING", "near miss", None),
            HandlerLevelCase("ERROR", "failure", "ERROR"),
            HandlerLevelCase("CRITICAL", "fatal", "CRITICAL"),
        ],
        ids=["trace", "debug", "info", "warning", "error", "critical"],
    )
    def test_error_handler_rejects_below_threshold(
        probe: AdapterProbe,
        case: HandlerLevelCase,
    ) -> None:
        """An ERROR handler should emit only ERROR and CRITICAL records.

        The femtologging logger is left at ``"TRACE"``, the most permissive
        threshold available, so any record that fails to appear can only have
        been rejected by the wrapped handler's own level.
        """
        output = probe.capture(
            LogRequest("app", case.level, case.message, logger_level="TRACE"),
            HandlerSetup(
                formatter=logging.Formatter("%(levelname)s %(message)s"),
                handler_level=logging.ERROR,
            ),
        )
        expected = (
            ""
            if case.expected_levelname is None
            else f"{case.expected_levelname} {case.message}\n"
        )
        assert output == expected, (
            f"handler at ERROR gave {output!r} for level {case.level!r}, "
            f"expected {expected!r}"
        )

    @staticmethod
    @pytest.mark.parametrize(
        "level",
        ["TRACE", "DEBUG", "INFO"],
        ids=["trace", "debug", "info"],
    )
    def test_notset_handler_accepts_every_level(
        probe: AdapterProbe,
        level: str,
    ) -> None:
        """A NOTSET handler should accept records at every mapped level.

        ``logging.NOTSET`` is numeric zero, which is falsy. A truthiness test
        in place of the documented comparison would treat it as an
        impossibly high threshold and silently drop everything, so TRACE -- the
        lowest mapped numeric level -- is the case that exposes that mistake.
        """
        output = probe.capture(
            LogRequest("app", level, "accepted", logger_level="TRACE"),
            HandlerSetup(
                formatter=logging.Formatter("%(message)s"),
                handler_level=logging.NOTSET,
            ),
        )
        assert output == "accepted\n", (
            f"NOTSET handler dropped a {level} record: {output!r}"
        )


class TestHandleRecordFilterAndLockContract:
    """Verify the level gate leaves the filter-and-lock path intact."""

    @staticmethod
    def test_handle_and_filter_counts_by_level() -> None:
        """Rejected records skip the handler; accepted ones run it once."""
        handler = CapturingHandler()
        handler.setLevel(logging.ERROR)
        counting_filter = CountingFilter()
        handler.addFilter(counting_filter)
        adapter = StdlibHandlerAdapter(handler)

        with mock.patch.object(handler, "handle", wraps=handler.handle) as handle_spy:
            dispatch(adapter, "DEBUG", "suppressed")
            assert handle_spy.call_count == 0, (
                f"rejected record still entered handle(): {handle_spy.call_count} calls"
            )
            assert counting_filter.calls == 0, (
                "rejected record still reached the filter: "
                f"{counting_filter.calls} calls"
            )
            assert not handler.records, (
                f"rejected record was emitted: {emitted_messages(handler)!r}"
            )

            dispatch(adapter, "ERROR", "accepted")
            assert handle_spy.call_count == 1, (
                "accepted record should enter handle() once: "
                f"{handle_spy.call_count} calls"
            )
            assert counting_filter.calls == 1, (
                "accepted record should consult the filter once: "
                f"{counting_filter.calls} calls"
            )
            assert emitted_messages(handler) == ["accepted"], (
                f"unexpected emitted records: {emitted_messages(handler)!r}"
            )

    @staticmethod
    def test_filter_rejection_still_runs_handle() -> None:
        """A record above the level is still subject to the handler's filters."""
        handler = CapturingHandler()
        handler.setLevel(logging.ERROR)
        rejecting_filter = CountingFilter(should_accept=False)
        handler.addFilter(rejecting_filter)
        adapter = StdlibHandlerAdapter(handler)

        with mock.patch.object(handler, "handle", wraps=handler.handle) as handle_spy:
            dispatch(adapter, "CRITICAL", "filtered out")

        assert handle_spy.call_count == 1, (
            f"an accepted record should still enter handle(): {handle_spy.call_count}"
        )
        assert rejecting_filter.calls == 1, (
            f"an accepted record should consult the filter once: "
            f"{rejecting_filter.calls}"
        )
        assert not handler.records, (
            f"a filter-rejected record must not be emitted: "
            f"{emitted_messages(handler)!r}"
        )
