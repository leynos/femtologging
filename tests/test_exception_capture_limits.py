"""Behavioural coverage for bounded exception payload capture.

The tests observe the public boundary only: ``FemtoLogger.log`` returns the
formatted text, and user-defined ``__str__``/``__repr__`` methods count their
own invocations. Counting is what distinguishes "captured nothing" from
"captured something and then dropped it", which formatting alone cannot show.

Every pathological exception is built from cheap pieces, so the whole module
runs well inside a second even though one message is two megabytes long.
"""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import (
    ConfigBuilder,
    FemtoLogger,
    LevelFilterBuilder,
    LoggerConfigBuilder,
    NameFilterBuilder,
    get_logger,
)

if typ.TYPE_CHECKING:
    import logging

    from femtologging._adapter_record import ExcInfo

_HUGE_LEN = 2 * 1024 * 1024
"""Length of the oversized strings and byte strings used below."""


class _HugeValue:
    """Value whose conversions build a huge string and count their calls."""

    str_calls = 0
    repr_calls = 0

    def __init__(self, size: int = _HUGE_LEN) -> None:
        """Store the length of the text each conversion returns."""
        self._size = size

    @classmethod
    def reset(cls) -> None:
        """Clear both call counters."""
        cls.str_calls = 0
        cls.repr_calls = 0

    def __str__(self) -> str:
        """Return a huge string, counting the call."""
        type(self).str_calls += 1
        return "s" * self._size

    def __repr__(self) -> str:
        """Return a huge representation, counting the call."""
        type(self).repr_calls += 1
        return "r" * self._size


class _CountedValue:
    """Value whose small representation counts its invocations."""

    repr_calls = 0

    @classmethod
    def reset(cls) -> None:
        """Clear the call counter."""
        cls.repr_calls = 0

    def __repr__(self) -> str:
        """Return a short representation, counting the call."""
        type(self).repr_calls += 1
        return "v"


class _ExplodingRepr:
    """Value whose representation raises, counting the attempt."""

    repr_calls = 0

    @classmethod
    def reset(cls) -> None:
        """Clear the call counter."""
        cls.repr_calls = 0

    def __repr__(self) -> str:
        """Raise, counting the attempt."""
        type(self).repr_calls += 1
        msg = "repr boom"
        raise RuntimeError(msg)

    def __str__(self) -> str:
        """Return a usable string so the message remains renderable."""
        return "safe"


def _log_once(
    logger: FemtoLogger, exc: BaseException, message: str = "bounded"
) -> str | None:
    """Log *exc* at INFO and return the formatted output."""
    return logger.log("INFO", message, exc_info=exc)


def _deep_cause_chain(depth: int) -> BaseException:
    """Build a cause chain iteratively, avoiding deep recursion in the test."""
    current: BaseException = ValueError("origin")
    for index in range(depth):
        wrapper = RuntimeError(f"level {index}")
        wrapper.__cause__ = current
        current = wrapper
    return current


def _nested_groups(depth: int) -> BaseException:
    """Build nested exception groups iteratively."""
    current: BaseException = ValueError("leaf")
    for index in range(depth):
        current = ExceptionGroup(f"group {index}", [current])
    return current


def test_huge_string_argument_is_truncated_with_markers() -> None:
    """A two-megabyte message argument must be bounded and reported."""
    _HugeValue.reset()
    logger = FemtoLogger("limits.huge-str")

    output = _log_once(logger, ValueError(_HugeValue()))

    assert output is not None, "an accepted record must produce formatted output"
    assert len(output) < 10_000, "formatted output must stay bounded"
    assert "message truncated" in output, "the message marker must be reported"
    assert "argument representations truncated" in output, (
        "the argument marker must be reported"
    )
    assert _HugeValue.str_calls == 1, "the message must stringify the value once"
    assert _HugeValue.repr_calls == 1, "the argument must be represented once"


def test_huge_byte_string_is_truncated() -> None:
    """A two-megabyte ``bytes`` argument must not be rendered in full."""
    logger = FemtoLogger("limits.huge-bytes")

    output = _log_once(logger, ValueError(b"b" * _HUGE_LEN))

    assert output is not None, "an accepted record must produce formatted output"
    assert len(output) < 10_000, "the formatted output must stay bounded"
    assert "..." in output, "the truncated representation must keep its suffix"


def test_failing_repr_is_contained_and_reported() -> None:
    """A raising ``__repr__`` must not escape the logging call."""
    _ExplodingRepr.reset()
    logger = FemtoLogger("limits.failing-repr")

    output = _log_once(logger, ValueError(_ExplodingRepr()))

    assert output is not None, "logging must not propagate a user ``__repr__`` failure"
    assert "repr boom" not in output, (
        "the contained failure must not leak into the output"
    )
    assert "conversion failures" in output, (
        "the summary must report the failed conversion"
    )
    assert _ExplodingRepr.repr_calls == 1, "the failing repr runs at most once"


def test_many_arguments_are_capped_and_counted() -> None:
    """A thousand arguments must cost at most the argument ceiling."""
    _CountedValue.reset()
    logger = FemtoLogger("limits.many-args")
    exc = ValueError(*[_CountedValue() for _ in range(1000)])

    output = _log_once(logger, exc)

    assert output is not None, "an accepted record must produce formatted output"
    expected_reprs = 32
    assert _CountedValue.repr_calls == expected_reprs, (
        "one representation per retained argument, never per supplied argument"
    )
    assert "968 arguments omitted" in output, "the argument ceiling must be reported"
    assert len(output) < 40_000, "the formatted output must stay bounded"


def test_a_shortened_argument_marks_the_message_as_truncated() -> None:
    """A message assembled from a shortened argument is itself shortened."""
    logger = FemtoLogger("limits.short-arg-message")

    output = logger.log("INFO", "joined", exc_info=ValueError("a" * 600, "b"))

    assert output is not None, "an accepted record must produce formatted output"
    assert "message truncated" in output, (
        "a message built from a shortened argument must report the cut"
    )


def test_many_notes_are_capped_and_counted() -> None:
    """A thousand notes must be bounded to the note ceiling."""
    logger = FemtoLogger("limits.many-notes")
    exc = ValueError("with notes")
    for index in range(1000):
        exc.add_note(f"note {index}")

    output = _log_once(logger, exc)

    assert output is not None, "an accepted record must produce formatted output"
    assert "1 notes omitted" in output, (
        "the note ceiling must be reported as at least one confirmed omission"
    )
    assert len(output) < 40_000, "the formatted output must stay bounded"


def test_deep_cause_chain_is_bounded_and_marked() -> None:
    """A two-hundred-link cause chain must stop at the chain ceiling."""
    logger = FemtoLogger("limits.deep-chain")

    output = _log_once(logger, _deep_cause_chain(200))

    assert output is not None, "an accepted record must produce formatted output"
    assert "... exception cause omitted ..." in output, (
        "the dropped cause must be reported"
    )
    assert len(output) < 200_000, "the formatted output must stay bounded"


def test_wide_exception_group_is_bounded_and_marked() -> None:
    """A hundred-member group must report the dropped members."""
    logger = FemtoLogger("limits.wide-group")
    exc = ExceptionGroup(
        "cluster", [ValueError(f"child {index}") for index in range(100)]
    )

    output = _log_once(logger, exc)

    assert output is not None, "an accepted record must produce formatted output"
    assert "and 85 more exceptions" in output, (
        "the dropped group members must be reported"
    )
    assert len(output) < 100_000, "the formatted output must stay bounded"


def test_deeply_nested_groups_are_bounded_and_marked() -> None:
    """Nested groups beyond the depth ceiling must be reported, not followed."""
    logger = FemtoLogger("limits.deep-group")

    output = _log_once(logger, _nested_groups(20))

    assert output is not None, "an accepted record must produce formatted output"
    assert "exception group members truncated" in output, (
        "the nested-group cut must be reported"
    )
    assert len(output) < 100_000, "the formatted output must stay bounded"


def _configure_with_filter(
    filter_name: str, filter_builder: object, logger_name: str
) -> FemtoLogger:
    """Install a single named filter on *logger_name* and return the logger."""
    (
        ConfigBuilder()
        .with_filter(filter_name, filter_builder)
        .with_logger(logger_name, LoggerConfigBuilder().with_filters([filter_name]))
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
        .build_and_init()
    )
    return get_logger(logger_name)


def _assert_rejected_without_stringification(
    filter_name: str, filter_builder: object, logger_name: str
) -> None:
    """Install *filter_builder* so INFO is rejected, then probe stringification."""
    logger = _configure_with_filter(filter_name, filter_builder, logger_name)
    _HugeValue.reset()

    output = _log_once(logger, ValueError(_HugeValue()))

    assert output is None, "the native filter must reject the record"
    assert _HugeValue.str_calls == 0, (
        "a record rejected by a metadata-only filter must not be captured"
    )
    assert _HugeValue.repr_calls == 0, (
        "a rejected record must not represent its argument"
    )


@pytest.mark.parametrize(
    ("rejecting", "accepting"),
    [
        (
            LevelFilterBuilder().with_max_level("DEBUG"),
            LevelFilterBuilder().with_max_level("INFO"),
        ),
        (
            NameFilterBuilder().with_prefix("elsewhere"),
            NameFilterBuilder().with_prefix("limits.native"),
        ),
    ],
    ids=["level-filter", "name-filter"],
)
def test_native_filter_rejection_skips_capture(
    rejecting: LevelFilterBuilder | NameFilterBuilder,
    accepting: LevelFilterBuilder | NameFilterBuilder,
) -> None:
    """A native filter that rejects INFO must avoid all capture work."""
    _assert_rejected_without_stringification("native", rejecting, "limits.native")

    # Positive control: the same logger name and level let an accepting native
    # filter pass INFO to capture. Without it, an earlier threshold check that
    # never consulted the filter would also produce the zero conversions above.
    logger = _configure_with_filter("native", accepting, "limits.native")
    _HugeValue.reset()

    output = _log_once(logger, ValueError(_HugeValue()))

    assert output is not None, "the accepting control must pass the record"
    assert _HugeValue.str_calls == 1, (
        "the accepting control must stringify the exception once"
    )


def test_below_threshold_record_is_not_captured() -> None:
    """A record below the logger level must not stringify its exception."""
    logger = FemtoLogger("limits.below-threshold")
    logger.set_level("ERROR")
    _HugeValue.reset()

    output = _log_once(logger, ValueError(_HugeValue()))

    assert output is None, "the record must be suppressed"
    assert _HugeValue.str_calls == 0, (
        "a rejected record must not stringify its exception"
    )
    assert _HugeValue.repr_calls == 0, (
        "a rejected record must not represent its argument"
    )


def test_rejecting_callback_sees_a_bounded_payload() -> None:
    """A callback filter runs after capture and must receive bounded data."""
    seen: dict[str, int | bool] = {}

    def reject(record: logging.LogRecord) -> bool:
        """Record the payload's marker and sizes, then reject the record."""
        payload = typ.cast("ExcInfo", record.exc_info)
        marker = payload.get("truncation", {})
        seen["message_truncated"] = marker.get("message_truncated", False)
        seen["args_truncated"] = marker.get("args_truncated", False)
        message = payload.get("message", "")
        seen["message_bytes"] = len(message.encode("utf-8"))
        args_repr = payload.get("args_repr", [])
        seen["arg_bytes"] = len(args_repr[0].encode("utf-8")) if args_repr else 0
        return False

    (
        ConfigBuilder()
        .with_filter("callback", reject)
        .with_logger(
            "limits.callback", LoggerConfigBuilder().with_filters(["callback"])
        )
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
        .build_and_init()
    )
    _HugeValue.reset()

    output = _log_once(get_logger("limits.callback"), ValueError(_HugeValue()))

    assert output is None, "the rejecting callback must drop the record"
    assert seen["message_truncated"] is True, (
        "the callback must observe the message marker"
    )
    assert seen["args_truncated"] is True, (
        "the callback must observe the argument marker"
    )
    # The marker alone does not prove the data is bounded: a callback handed
    # both two-megabyte strings would still see the flags set.
    message_bytes = seen["message_bytes"]
    assert 0 < message_bytes <= 4096, (
        f"the callback received a {message_bytes}-byte message"
    )
    arg_bytes = seen["arg_bytes"]
    assert 0 < arg_bytes <= 512, (
        f"the callback received a {arg_bytes}-byte argument representation"
    )
    assert _HugeValue.str_calls == 1, "the accepted record must stringify once"
    assert _HugeValue.repr_calls == 1, (
        "the accepted record must represent its argument once"
    )
