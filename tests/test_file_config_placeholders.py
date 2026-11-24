r"""Tests for placeholder scanning and raw handler parameter limits.

The earlier implementation expanded ``%(name)s`` with ``re.sub`` and the
pattern ``%\(([^)]+)\)s``. Its name group admits nested ``%(`` openers, so
many unterminated openers made the engine restart at every opener and rescan
an overlapping suffix: quadratic work on malformed input. These tests pin the
linear scanner's semantics, count searched spans through an injected ``find``
seam rather than timing anything, and cover the raw handler parameter limit.
"""

from __future__ import annotations

import re
import typing as typ

import pytest

from femtologging._placeholders import _expand_placeholders, _iter_placeholders
from femtologging.file_config import (
    _MAX_HANDLER_PARAMETER_CHARS,
    _check_parameter_size,
    _ini_to_dict_config,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

_ORACLE = re.compile(r"%\(([^)]+)\)s")

_PLACEHOLDER_CASES = [
    pytest.param("()", {}, "()", id="no-placeholder"),
    pytest.param(
        "('%(dir)s/app.log',)",
        {"dir": "/var/log"},
        "('/var/log/app.log',)",
        id="single-substitution",
    ),
    pytest.param("%(a)s%(b)s", {"a": "1", "b": "2"}, "12", id="adjacent"),
    pytest.param("%(a)s/%(a)s", {"a": "x"}, "x/x", id="repeated"),
    pytest.param("%(naïve)s", {"naïve": "✓"}, "✓", id="unicode-name"),
    pytest.param("%(line\nbreak)s", {"line\nbreak": "ok"}, "ok", id="newline-name"),
    pytest.param(
        "%(with space)s", {"with space": "ok"}, "ok", id="space-containing-name"
    ),
    pytest.param("100%% done", {}, "100%% done", id="percent-signs-untouched"),
    pytest.param("%%(a)s", {"a": "x"}, "%x", id="no-percent-escaping"),
    pytest.param("%(a", {}, "%(a", id="missing-closer"),
    pytest.param("%(a)", {}, "%(a)", id="missing-suffix"),
    pytest.param("%()s", {}, "%()s", id="empty-name"),
    pytest.param("%(x)%(a)s", {"a": "A"}, "%(x)A", id="failed-candidate-then-valid"),
    pytest.param(
        "%(outer%(inner)s",
        {"outer%(inner": "X"},
        "X",
        id="nested-opener-in-name",
    ),
    pytest.param(
        "%(a)s",
        {"a": "%(b)s", "b": "B"},
        "%(b)s",
        id="replacement-not-rescanned",
    ),
]

# Each builder constructs one input shape whose repeated unit stays well below
# the handler parameter limit at the largest tested size.
_SCAN_BUILDERS = [
    pytest.param(lambda size: "%(" * size, id="unterminated-openers"),
    pytest.param(lambda size: "%(a)" * size, id="failed-closers"),
    pytest.param(lambda size: "%(x" * size + ")s", id="nested-openers"),
    pytest.param(lambda size: "%(a)s" * size, id="adjacent-valid"),
]


class _RecordingFind:
    """A ``str.find`` replacement recording call count and searched span."""

    def __init__(self, value: str) -> None:
        """Record searches against *value*."""
        self._value = value
        self.calls = 0
        self.searched = 0

    def __call__(self, substring: str, start: int) -> int:
        """Return ``str.find``'s result while accumulating the searched span."""
        index = self._value.find(substring, start)
        self.calls += 1
        if index < 0:
            self.searched += len(self._value) - start
        else:
            self.searched += index + len(substring) - start
        return index


def _config_sections(
    *,
    args: str | None = None,
    kwargs: str | None = None,
) -> list[tuple[str, list[tuple[str, str]]]]:
    """Return parsed INI pairs for a root logger with one StreamHandler."""
    handler_entries: list[tuple[str, str]] = [
        ("class", "femtologging.StreamHandler"),
    ]
    if args is not None:
        handler_entries.append(("args", args))
    if kwargs is not None:
        handler_entries.append(("kwargs", kwargs))
    return [
        ("loggers", [("keys", "root")]),
        ("handlers", [("keys", "h")]),
        ("handler_h", handler_entries),
        ("logger_root", [("handlers", "h")]),
    ]


def _assert_matches_oracle(value: str) -> None:
    """Assert the scanner's spans and names agree with ``finditer``."""
    expected = [
        (match.start(), match.end(), match.group(1))
        for match in _ORACLE.finditer(value)
    ]
    assert list(_iter_placeholders(value)) == expected, (
        "scanner spans and names must match the regex oracle"
    )


@pytest.mark.parametrize(("value", "defaults", "expected"), _PLACEHOLDER_CASES)
def test_expand_placeholders_matches_the_documented_semantics(
    value: str, defaults: dict[str, str], expected: str
) -> None:
    """Valid, malformed, and nested forms expand exactly as documented."""
    assert _expand_placeholders(value, defaults) == expected, (
        f"expanding {value!r} must yield {expected!r}"
    )


def test_expand_placeholders_reports_the_first_unknown_name() -> None:
    """The error quotes the first unknown name and the original value."""
    value = "%(known)s then %(missing)s then %(also_missing)s"

    with pytest.raises(ValueError, match="unknown placeholder 'missing'") as raised:
        _expand_placeholders(value, {"known": "K"})

    expected = f"unknown placeholder 'missing' in {value!r}"
    assert str(raised.value) == expected, (
        "the diagnostic must quote the first unknown name and the input"
    )


def test_expand_placeholders_handles_many_unterminated_openers() -> None:
    """Input just below the limit with no closers passes through unchanged."""
    value = "%(" * (_MAX_HANDLER_PARAMETER_CHARS // 2 - 1)
    assert len(value) < _MAX_HANDLER_PARAMETER_CHARS, (
        "the fixture must stay inside the handler parameter limit"
    )

    assert _expand_placeholders(value, {}) == value, (
        "unterminated openers must pass through unchanged"
    )


@pytest.mark.parametrize("build_value", _SCAN_BUILDERS)
@pytest.mark.parametrize("size", [1_000, 10_000, 30_000])
def test_placeholder_search_work_is_bounded(
    build_value: cabc.Callable[[int], str], size: int
) -> None:
    """The scanner searches at most one input length regardless of shape.

    The old regex restarted at every ``%(`` because its name group accepted
    nested openers, so repeated malformed openers rescanned overlapping
    suffixes. Summing searched spans proves the bounded work without a timing
    assertion.
    """
    value = build_value(size)
    recorder = _RecordingFind(value)

    matches = list(_iter_placeholders(value, find=recorder))

    assert recorder.searched <= len(value), (
        f"{recorder.searched} searched characters exceed the "
        f"{len(value)}-character input"
    )
    assert recorder.calls <= len(value) + 2, (
        f"{recorder.calls} searches exceed the {len(value) + 2} allowed"
    )
    assert matches == [
        (match.start(), match.end(), match.group(1))
        for match in _ORACLE.finditer(value)
    ], "the bounded scan must still find the oracle's placeholders"


@pytest.mark.parametrize("field", ["args", "kwargs"])
def test_check_parameter_size_accepts_exactly_the_limit(field: str) -> None:
    """A raw value at the limit is accepted."""
    _check_parameter_size("h", field, "x" * _MAX_HANDLER_PARAMETER_CHARS)


@pytest.mark.parametrize("field", ["args", "kwargs"])
def test_check_parameter_size_rejects_one_code_point_over_the_limit(
    field: str,
) -> None:
    """One code point over the limit raises without echoing the value."""
    value = "MARKER" + "x" * _MAX_HANDLER_PARAMETER_CHARS

    with pytest.raises(ValueError, match="characters; the limit is") as raised:
        _check_parameter_size("h", field, value)

    message = str(raised.value)
    assert f"handler 'h' {field} has {len(value)} characters" in message, (
        "the diagnostic must name the handler, field and actual length"
    )
    assert f"the limit is {_MAX_HANDLER_PARAMETER_CHARS}" in message, (
        "the diagnostic must state the configured limit"
    )
    assert "MARKER" not in message, "the value must not be echoed"


def test_check_parameter_size_rejects_a_megabyte_without_echoing_it() -> None:
    """An approximately one-megabyte value is rejected without echo."""
    value = "MARKER" + "x" * (1024 * 1024)

    with pytest.raises(ValueError, match="characters; the limit is") as raised:
        _check_parameter_size("h", "args", value)

    message = str(raised.value)
    assert str(len(value)) in message, "the diagnostic must state the actual length"
    assert "MARKER" not in message, "the value must not be echoed"


@pytest.mark.parametrize("field", ["args", "kwargs"])
def test_ini_to_dict_config_accepts_a_parameter_at_the_limit(field: str) -> None:
    """Translation accepts a raw parameter of exactly the allowed size."""
    value = "x" * _MAX_HANDLER_PARAMETER_CHARS
    sections = (
        _config_sections(args=value)
        if field == "args"
        else _config_sections(kwargs=value)
    )

    config = _ini_to_dict_config(sections, None, disable_existing=True)

    assert config["handlers"]["h"][field] == value, (
        "a value at the limit must survive translation unchanged"
    )


@pytest.mark.parametrize("field", ["args", "kwargs"])
def test_ini_to_dict_config_rejects_a_parameter_over_the_limit(field: str) -> None:
    """Translation rejects one code point more with a full diagnostic."""
    value = "x" * (_MAX_HANDLER_PARAMETER_CHARS + 1)
    sections = (
        _config_sections(args=value)
        if field == "args"
        else _config_sections(kwargs=value)
    )

    with pytest.raises(ValueError, match="characters; the limit is") as raised:
        _ini_to_dict_config(sections, None, disable_existing=True)

    message = str(raised.value)
    assert f"handler 'h' {field} has {len(value)} characters" in message, (
        "the diagnostic must name the handler, field and actual length"
    )
    assert f"the limit is {_MAX_HANDLER_PARAMETER_CHARS}" in message, (
        "the diagnostic must state the configured limit"
    )
    assert value not in message, "the oversized value must not be echoed"


def test_ini_to_dict_config_rejects_an_oversized_megabyte_parameter() -> None:
    """A one-megabyte raw value is rejected with the usual diagnostic."""
    value = "MARKER" + "x" * (1024 * 1024)
    sections = _config_sections(args=value)

    with pytest.raises(ValueError, match="characters; the limit is") as raised:
        _ini_to_dict_config(sections, None, disable_existing=True)

    message = str(raised.value)
    assert "handler 'h' args" in message, (
        "the diagnostic must name the handler and field"
    )
    assert str(len(value)) in message, "the diagnostic must state the actual length"
    assert "MARKER" not in message, "the value must not be echoed"


def test_ini_to_dict_config_checks_size_before_placeholder_names() -> None:
    """An oversized value with an unknown placeholder reports the size error."""
    value = "%(missing)s" + "x" * _MAX_HANDLER_PARAMETER_CHARS
    sections = _config_sections(kwargs=value)

    with pytest.raises(ValueError, match="characters; the limit is") as raised:
        _ini_to_dict_config(sections, None, disable_existing=True)

    assert "unknown placeholder" not in str(raised.value), (
        "the size error must take precedence over placeholder names"
    )


@pytest.mark.parametrize(
    "value",
    [
        pytest.param("plain text", id="plain"),
        pytest.param("%(a)s", id="single"),
        pytest.param("%(a)s%(b)s", id="adjacent"),
        pytest.param("%(a", id="unterminated"),
        pytest.param("%(outer%(inner)s", id="nested"),
        pytest.param("%()s", id="empty-name"),
    ],
)
def test_iter_placeholders_matches_regex_finditer(value: str) -> None:
    """Scanner offsets and names agree with the regex oracle."""
    _assert_matches_oracle(value)
