"""Tests for the filter snapshot under declarative-only ``dictConfig``.

``dictConfig`` reads the filters section once and passes that snapshot to
construction, so a mapping that changes between reads cannot introduce a
factory after the declarative-only preflight. The preflight detects ``"()"``
by iterating keys, so it never reads a factory value.
"""

from __future__ import annotations

import collections.abc as cabc
import typing as typ

import pytest

from femtologging import dictConfig, get_logger
from femtologging._config_filters import build_filter_from_dict

_FILTER_ERROR_PATTERN = r"filter 'f'.*allow_factories=False"


class _ShiftingFiltersConfig(cabc.Mapping[str, object]):
    """Return declarative filters on the first read, then a factory entry."""

    def __init__(self) -> None:
        """Start with no filter reads and an uninvoked factory sentinel."""
        self.filter_reads = 0
        self.factory_invocations = 0

    def _factory(self, **_kwargs: object) -> object:
        """Record any invocation of the factory that must stay unreachable."""
        self.factory_invocations += 1
        return lambda _record: True

    def _filters_section(self) -> dict[str, object]:
        """Return declarative filters once, then a factory entry."""
        self.filter_reads += 1
        if self.filter_reads > 1:
            return {"f": {"()": self._factory}}
        return {"lvl": {"level": "INFO"}}

    @typ.override
    def __getitem__(self, key: str) -> object:
        """Serve the fixed sections and the shifting filters section."""
        fixed: dict[str, object] = {
            "version": 1,
            "loggers": {"app": {"filters": ["lvl"]}},
            "root": {"level": "DEBUG"},
        }
        if key == "filters":
            return self._filters_section()
        if key in fixed:
            return fixed[key]
        raise KeyError(key)

    @typ.override
    def __iter__(self) -> cabc.Iterator[str]:
        """Iterate the configured top-level keys."""
        return iter(("version", "filters", "loggers", "root"))

    @typ.override
    def __len__(self) -> int:
        """Report the number of configured top-level keys."""
        return 4


def test_declarative_mode_reads_filters_once() -> None:
    """Preflight and construction share a single filters read."""
    config = _ShiftingFiltersConfig()
    dictConfig(config, allow_factories=False)
    assert config.filter_reads == 1, "the filters section must be read once"
    assert config.factory_invocations == 0, "the later factory must never run"
    app = get_logger("app")
    assert app.log("ERROR", "filtered") is None, (
        "the collected INFO filter must reject a record above its level"
    )
    assert app.log("INFO", "allowed") is not None, (
        "records at the filter level must pass the collected filter"
    )


class _UnreadableFactoryEntry(cabc.Mapping[str, object]):
    """Expose a ``"()"`` key whose value must never be read."""

    def __init__(self) -> None:
        """Start without any value access."""
        self.value_reads = 0

    @typ.override
    def __getitem__(self, key: str) -> object:
        """Fail if the policy check reads the factory value."""
        self.value_reads += 1
        msg = f"factory value for {key!r} must not be read"
        raise AssertionError(msg)

    @typ.override
    def __iter__(self) -> cabc.Iterator[str]:
        """Iterate the single factory key."""
        return iter(("()",))

    @typ.override
    def __len__(self) -> int:
        """Report the single configured key."""
        return 1


def test_declarative_mode_rejects_by_key_without_reading_value() -> None:
    """Rejection inspects keys only, even when the value accessor raises."""
    entry = _UnreadableFactoryEntry()
    config = {"version": 1, "filters": {"f": entry}, "root": {"level": "DEBUG"}}
    with pytest.raises(ValueError, match=_FILTER_ERROR_PATTERN):
        dictConfig(config, allow_factories=False)
    assert entry.value_reads == 0, "the factory value must never be read"


def test_build_filter_from_dict_rejects_without_reading_value() -> None:
    """The direct guard rejects by key before copying the entry."""
    entry = _UnreadableFactoryEntry()
    unreadable = typ.cast("dict[str, object]", entry)
    with pytest.raises(ValueError, match=_FILTER_ERROR_PATTERN):
        build_filter_from_dict("f", unreadable, allow_factories=False)
    assert entry.value_reads == 0, "the factory value must never be read"
