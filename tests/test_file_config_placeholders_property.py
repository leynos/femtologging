r"""Property tests comparing the placeholder scanner with the old regex.

``tests/test_file_config_placeholders.py`` pins the scanner's semantics with
named and parameterized cases. Those cases sample relations that should hold
over the whole input space, so they are stated here as properties instead: the
linear scanner yields exactly the spans and names that ``finditer`` reports,
and expansion produces the same text or the same diagnostic as the previous
``re.sub`` implementation.
"""

from __future__ import annotations

import re
import typing as typ

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from femtologging._placeholders import _expand_placeholders, _iter_placeholders

if typ.TYPE_CHECKING:
    import collections.abc as cabc

_PER_PLACEHOLDER_ORACLE = re.compile(r"%\(([^)]+)\)s")

_ALPHABET = st.sampled_from(("%", "(", ")", "s", "a", "b", " ", "\n", "é", "-"))

_PLAIN_TEXT = st.text(alphabet=_ALPHABET, max_size=40)

# Hypothesis requires length-one alphabet elements, so the nested opener the
# name group can absorb is built from separate `%` and `(` characters here.
_NAME = st.text(
    alphabet=st.sampled_from(("a", "b", " ", "é", "%", "(", "\n")),
    min_size=1,
    max_size=8,
)

_FRAGMENTS = st.one_of(
    _PLAIN_TEXT,
    st.just("%%"),
    st.just("%("),
    st.just(")"),
    st.just(")s"),
    st.just("%()s"),  # empty name
    st.just("%(a)d"),  # wrong suffix
    st.just("%(a)"),  # no suffix at all
    st.just("%(%(a)s"),  # nested opener resolved by one closer
    st.tuples(st.just("%("), _NAME, st.just(")s")).map("".join),
    st.tuples(st.just("%("), _NAME, st.just(")d")).map("".join),
    st.tuples(st.just("%("), st.just("%("), _NAME, st.just(")s")).map("".join),
)

_STRUCTURED_TEXT = st.lists(_FRAGMENTS, max_size=6).map("".join)

_KEYS = ("a", "b", "space key", "naïve", "nested%(key")
_DEFAULT_VALUES = ("plain", "%(a)s", "%%", "line\nbreak", "")
_DEFAULTS = st.dictionaries(
    keys=st.sampled_from(_KEYS),
    values=st.sampled_from(_DEFAULT_VALUES),
    max_size=4,
)


def _oracle_expand(value: str, defaults: cabc.Mapping[str, str]) -> str:
    """Expand *value* with the previous regex implementation as an oracle."""

    def replacer(match: re.Match[str]) -> str:
        """Resolve one regex match against *defaults*."""
        key = match.group(1)
        if key not in defaults:
            msg = f"unknown placeholder {key!r} in {value!r}"
            raise ValueError(msg)
        return defaults[key]

    return _PER_PLACEHOLDER_ORACLE.sub(replacer, value)


def _oracle_outcome(
    value: str, defaults: cabc.Mapping[str, str]
) -> tuple[str | None, str | None]:
    """Return the oracle's expanded text, or its error message."""
    try:
        return _oracle_expand(value, defaults), None
    except ValueError as oracle_error:
        return None, str(oracle_error)


@settings(max_examples=200, deadline=None)
@given(value=_STRUCTURED_TEXT, defaults=_DEFAULTS)
def test_expansion_agrees_with_the_regex_oracle(
    value: str, defaults: dict[str, str]
) -> None:
    """Expansion yields the oracle's text or raises its diagnostic."""
    expected, expected_error = _oracle_outcome(value, defaults)

    if expected_error is None:
        assert expected is not None, "the oracle reports text only with no error"
        assert _expand_placeholders(value, defaults) == expected, (
            f"expanding {value!r} with {defaults!r} must yield the oracle's text"
        )
    else:
        with pytest.raises(ValueError, match=re.escape(expected_error)) as raised:
            _expand_placeholders(value, defaults)
        assert str(raised.value) == expected_error, (
            "the diagnostic must match the oracle exactly, with no extra text"
        )


@settings(max_examples=200, deadline=None)
@given(value=st.one_of(_PLAIN_TEXT, _STRUCTURED_TEXT))
def test_scanner_matches_regex_finditer(value: str) -> None:
    """Span offsets and names agree with ``finditer`` on generated input."""
    expected = [
        (match.start(), match.end(), match.group(1))
        for match in _PER_PLACEHOLDER_ORACLE.finditer(value)
    ]

    assert list(_iter_placeholders(value)) == expected, (
        f"scanner spans and names must match finditer on {value!r}"
    )
