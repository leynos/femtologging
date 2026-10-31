"""Regression tests for Python callbacks during worker shutdown."""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import (
    FileHandlerBuilder,
    RotatingFileHandlerBuilder,
    StreamHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path


class _ClosableHandler(typ.Protocol):
    """Python methods shared by the worker-backed handler types."""

    def handle(self, logger: str, level: str, message: str) -> None: ...

    def flush(self) -> bool: ...

    def close(self) -> None: ...


def _make_reentrant_formatter(
    handler_ref: list[_ClosableHandler],
    handle_outcomes: list[str],
    flush_outcomes: list[bool | str],
) -> cabc.Callable[[cabc.Mapping[str, object]], str]:
    """Build a formatter that records re-entrant calls during shutdown."""

    def format_record(record: cabc.Mapping[str, object]) -> str:
        if handler_ref:
            try:
                handler_ref[0].handle("callback", "INFO", "during-close")
            except (RuntimeError, ValueError) as error:
                handle_outcomes.append(str(error))
            else:
                handle_outcomes.append("accepted")

            try:
                flush_outcomes.append(handler_ref[0].flush())
            except RuntimeError as error:
                flush_outcomes.append(str(error))
        return str(record.get("message", ""))

    return format_record


def _build_handler(
    handler_kind: str,
    log_path: Path,
    formatter: cabc.Callable[[cabc.Mapping[str, object]], str],
) -> _ClosableHandler:
    """Build a worker-backed handler with the supplied formatter."""
    match handler_kind:
        case "stream":
            built_handler = (
                StreamHandlerBuilder.stderr().with_formatter(formatter).build()
            )
        case "file":
            built_handler = (
                FileHandlerBuilder(str(log_path)).with_formatter(formatter).build()
            )
        case "rotating":
            built_handler = (
                RotatingFileHandlerBuilder(str(log_path))
                .with_formatter(formatter)
                .build()
            )
        case _:
            built_handler = (
                TimedRotatingFileHandlerBuilder(str(log_path))
                .with_formatter(formatter)
                .build()
            )
    return typ.cast("_ClosableHandler", built_handler)


@pytest.mark.parametrize(
    "handler_kind",
    ["stream", "file", "rotating", "timed-rotating"],
    ids=["stream", "file", "rotating", "timed-rotating"],
)
def test_close_allows_worker_formatter_to_reach_closed_state(
    tmp_path: Path,
    handler_kind: str,
) -> None:
    """Worker callbacks can reach closed-state handling while close waits."""
    handler_ref: list[_ClosableHandler] = []
    handle_outcomes: list[str] = []
    flush_outcomes: list[bool | str] = []
    log_path = tmp_path / f"{handler_kind}.log"
    formatter = _make_reentrant_formatter(
        handler_ref,
        handle_outcomes,
        flush_outcomes,
    )
    handler = _build_handler(handler_kind, log_path, formatter)
    handler_ref.append(handler)
    try:
        handler.handle("initial", "INFO", "queued")
        handler.close()
    finally:
        handler.close()

    assert len(handle_outcomes) == 1, "worker formatter should re-enter handle once"
    assert "handler is closed" in handle_outcomes[0], handle_outcomes[0]
    assert flush_outcomes == [False], flush_outcomes
