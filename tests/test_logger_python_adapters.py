"""Regression tests for raw PyO3 logger method adapters."""

from __future__ import annotations

import collections.abc as cabc
import inspect
import typing as typ

import pytest

from femtologging import FemtoLogger

type LogCall = cabc.Callable[[], object]


def assert_invalid_calls_raise_type_error(invalid_calls: list[LogCall]) -> None:
    """Assert each call rejected by the public Python signature raises TypeError."""
    for invalid_call in invalid_calls:
        with pytest.raises(TypeError):
            invalid_call()


def test_log_adapter_preserves_python_call_contract() -> None:
    """Raw PyO3 parsing retains the public ``log`` signature and defaults."""
    logger = FemtoLogger("contract")
    # Invalid calls intentionally exercise the runtime boundary beyond the stub.
    log_call = typ.cast("cabc.Callable[..., str | None]", logger.log)

    assert str(inspect.signature(FemtoLogger.log)) == (
        "(self, level, message, /, *, exc_info=None, stack_info=False)"
    ), "log must retain its documented positional and keyword-only parameters"
    assert log_call("INFO", "defaults") == "contract [INFO] defaults", (
        "log must retain its default option values"
    )
    assert log_call("INFO", "none options", exc_info=None, stack_info=None) == (
        "contract [INFO] none options"
    ), "log must preserve explicit None options"

    assert_invalid_calls_raise_type_error([
        lambda: log_call("INFO"),
        lambda: log_call("INFO", "message", "unexpected positional option"),
        lambda: log_call(level="INFO", message="keyword level"),
        lambda: log_call("INFO", "message", level="ERROR"),
        lambda: log_call("INFO", "message", message="duplicate message"),
        lambda: log_call("INFO", "message", unknown=True),
        lambda: log_call("INFO", "message", stack_info="true"),
    ])


def test_convenience_adapter_preserves_python_call_contract() -> None:
    """Raw adapters retain the public convenience-method signature and defaults."""
    logger = FemtoLogger("adapter.contract")
    info_call = typ.cast("cabc.Callable[..., str | None]", logger.info)

    assert str(inspect.signature(FemtoLogger.info)) == (
        "(self, message, /, *, exc_info=None, stack_info=False)"
    ), "info must retain its documented positional and keyword-only parameters"
    assert info_call("defaults") == "adapter.contract [INFO] defaults", (
        "info must retain its default option values"
    )
    assert info_call("none options", exc_info=None, stack_info=None) == (
        "adapter.contract [INFO] none options"
    ), "info must preserve explicit None options"

    assert_invalid_calls_raise_type_error([
        info_call,
        lambda: info_call("message", "unexpected positional option"),
        lambda: info_call(message="keyword message"),
        lambda: info_call("message", message="duplicate message"),
        lambda: info_call("message", unknown=True),
        lambda: info_call("message", stack_info="true"),
    ])
