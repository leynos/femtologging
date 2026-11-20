"""Tests that the default formatter frames a multiline message on one line."""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import FemtoFileHandler, get_logger

if typ.TYPE_CHECKING:
    from pathlib import Path


@pytest.mark.parametrize(
    ("message", "escaped"),
    [
        pytest.param(
            "forged\nother [ERROR] second",
            "forged\\nother [ERROR] second",
            id="line-feed",
        ),
        pytest.param(
            "forged\u000bother [ERROR] second",
            "forged\\u{b}other [ERROR] second",
            id="vertical-tab",
        ),
        pytest.param(
            "forged\u000cother [ERROR] second",
            "forged\\u{c}other [ERROR] second",
            id="form-feed",
        ),
        pytest.param(
            "forged\rother [ERROR] second",
            "forged\\rother [ERROR] second",
            id="carriage-return",
        ),
        pytest.param(
            "forged\r\nother [ERROR] second",
            "forged\\r\\nother [ERROR] second",
            id="carriage-return-line-feed",
        ),
        pytest.param(
            "forged\u001cother [ERROR] second",
            "forged\\u{1c}other [ERROR] second",
            id="file-separator",
        ),
        pytest.param(
            "forged\u001dother [ERROR] second",
            "forged\\u{1d}other [ERROR] second",
            id="group-separator",
        ),
        pytest.param(
            "forged\u001eother [ERROR] second",
            "forged\\u{1e}other [ERROR] second",
            id="record-separator",
        ),
        pytest.param(
            "forged\u0085other [ERROR] second",
            "forged\\u{85}other [ERROR] second",
            id="next-line",
        ),
        pytest.param(
            "forged\u2028other [ERROR] second",
            "forged\\u{2028}other [ERROR] second",
            id="line-separator",
        ),
        pytest.param(
            "forged\u2029other [ERROR] second",
            "forged\\u{2029}other [ERROR] second",
            id="paragraph-separator",
        ),
        pytest.param("trailing\n", "trailing\\n", id="trailing-line-feed"),
    ],
)
def test_default_formatter_escapes_forged_record_boundaries(
    tmp_path: Path, message: str, escaped: str
) -> None:
    """A message cannot add a line that mimics another logger's record."""
    path = tmp_path / "escaped.log"
    logger = get_logger("core")
    handler = FemtoFileHandler(str(path), capacity=8, flush_interval=1)
    logger.add_handler(handler)
    try:
        rendered = logger.info(message)
        assert rendered == f"core [INFO] {escaped}", (
            "the logger's formatted string must escape message line separators"
        )
        assert logger.flush_handlers() is True, "logger flush must succeed"
        assert handler.flush() is True, "handler flush must succeed"
    finally:
        handler.close()

    written = path.read_bytes()
    assert written == f"core [INFO] {escaped}\n".encode(), (
        "file output must hold one escaped record followed by a single terminator"
    )
    assert written.count(b"\n") == 1, "the record must occupy one physical line"
    assert b"\r" not in written, "file output must not preserve a carriage return"


def _raise_diagnostic() -> None:
    """Raise a ValueError so the caller can capture exception state."""
    message = "diagnostic boom"
    raise ValueError(message)


def test_default_formatter_keeps_diagnostics_after_escaped_message(
    tmp_path: Path,
) -> None:
    """Escaping the message leaves stack and traceback payloads intact."""
    path = tmp_path / "diagnostics.log"
    logger = get_logger("core")
    handler = FemtoFileHandler(str(path), capacity=8, flush_interval=1)
    logger.add_handler(handler)
    try:
        try:
            _raise_diagnostic()
        except ValueError:
            rendered = logger.info(
                "failed\ncore [CRITICAL] forged",
                exc_info=True,
                stack_info=True,
            )
        assert rendered is not None, "an enabled logger must return its formatted text"
        assert logger.flush_handlers() is True, "logger flush must succeed"
        assert handler.flush() is True, "handler flush must succeed"
    finally:
        handler.close()

    first_line, _, diagnostics = rendered.partition("\n")
    assert first_line == "core [INFO] failed\\ncore [CRITICAL] forged", (
        "the escaped message must be the whole first physical line"
    )
    stack_index = diagnostics.index("Stack (most recent call last):")
    traceback_index = diagnostics.index("Traceback (most recent call last):")
    value_error_index = diagnostics.index("ValueError: diagnostic boom")
    assert stack_index < traceback_index < value_error_index, (
        "the stack must precede the traceback and the exception line"
    )
    assert "test_default_formatter_keeps_diagnostics_after_escaped_message" in (
        diagnostics
    ), "the traceback must retain the raising test frame"
    assert "\ncore [CRITICAL] forged" not in rendered, (
        "the message must not introduce a line that looks like another record"
    )
    assert path.read_bytes() == rendered.encode() + b"\n", (
        "the file must hold the same diagnostic structure as the returned string"
    )
