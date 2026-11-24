"""Tests for declarative-only ``dictConfig`` filter handling.

``allow_factories=False`` rejects filter factories only; it applies no trust
policy to handler classes, destinations, or the rest of the document. These
tests prove that the mode accepts built-in ``level`` and ``name`` filters and
leaves default factory behaviour unchanged; the factory-rejection sentinels
live in ``tests/test_dict_config_factory_rejection.py``.
"""

from __future__ import annotations

import collections.abc as cabc
import dataclasses
import typing as typ

import pytest

from femtologging import dictConfig, get_logger
from tests.python_filter_support import (
    ContextFilterFactory,
    RecordCollector,
    wait_for,
)

_FACTORY_PATH = "tests.python_filter_support.ContextFilterFactory"


@dataclasses.dataclass(frozen=True, slots=True)
class _DeclarativeCase:
    """A declarative-only configuration plus the records it accepts or blocks."""

    config: dict[str, object]
    allowed_logger: str
    blocked_logger: str
    blocked_level: str


@pytest.mark.parametrize(
    "case",
    [
        _DeclarativeCase(
            {
                "version": 1,
                "filters": {"lvl": {"level": "INFO"}},
                "loggers": {"app": {"filters": ["lvl"]}},
                "root": {"level": "DEBUG"},
            },
            allowed_logger="app",
            blocked_logger="app",
            blocked_level="ERROR",
        ),
        _DeclarativeCase(
            {
                "version": 1,
                "filters": {"ns": {"name": "app"}},
                "loggers": {"app": {"filters": ["ns"]}, "other": {"filters": ["ns"]}},
                "root": {"level": "DEBUG"},
            },
            allowed_logger="app",
            blocked_logger="other",
            blocked_level="INFO",
        ),
        _DeclarativeCase(
            {
                "version": 1,
                "filters": {"lvl": {"level": "INFO"}},
                "root": {"level": "DEBUG", "filters": ["lvl"]},
            },
            allowed_logger="root",
            blocked_logger="root",
            blocked_level="ERROR",
        ),
        _DeclarativeCase(
            {
                "version": 1,
                "filters": {"ns": {"name": "app"}},
                "loggers": {"app": {"filters": ["ns"]}},
                "root": {"level": "DEBUG", "filters": ["ns"]},
            },
            allowed_logger="app",
            blocked_logger="root",
            blocked_level="INFO",
        ),
    ],
    ids=[
        "logger-level-filter",
        "logger-name-filter",
        "root-level-filter",
        "root-name-filter",
    ],
)
def test_declarative_mode_accepts_builtin_filters(case: _DeclarativeCase) -> None:
    """Declarative-only mode keeps built-in level and name filters working."""
    dictConfig(case.config, allow_factories=False)

    allowed = get_logger(case.allowed_logger).log("INFO", "allowed")
    assert allowed is not None, (
        f"logger {case.allowed_logger!r} should emit INFO in declarative-only mode"
    )
    blocked = get_logger(case.blocked_logger).log(case.blocked_level, "blocked")
    assert blocked is None, (
        f"logger {case.blocked_logger!r} should suppress {case.blocked_level}"
    )


def test_declarative_mode_accepts_config_without_filters() -> None:
    """Declarative-only mode does not require a filters section."""
    dictConfig({"version": 1, "root": {"level": "DEBUG"}}, allow_factories=False)
    assert get_logger("root").log("INFO", "enabled") is not None, (
        "a configuration without filters must remain usable in declarative-only mode"
    )


@pytest.mark.parametrize(
    ("filters_section", "msg"),
    [
        ([], r"filters must be a mapping"),
        ({1: {"level": "INFO"}}, r"filter ids must be strings"),
        ({"f": "not-a-mapping"}, r"filter config must be a mapping"),
    ],
    ids=["section-not-a-mapping", "non-string-id", "entry-not-a-mapping"],
)
def test_declarative_mode_preserves_structural_errors(
    filters_section: object, msg: str
) -> None:
    """Malformed filter sections still raise their usual structural errors."""
    config = {"version": 1, "filters": filters_section, "root": {"level": "DEBUG"}}
    with pytest.raises(TypeError, match=msg):
        dictConfig(config, allow_factories=False)


class _UnreadableConfig(cabc.Mapping[str, object]):
    """Fail if ``dictConfig`` reads the configuration at all."""

    @typ.override
    def __getitem__(self, key: str) -> object:
        """Fail on any key lookup."""
        msg = f"policy validation must not read the configuration ({key!r})"
        raise AssertionError(msg)

    @typ.override
    def __iter__(self) -> cabc.Iterator[str]:
        """Fail on any iteration."""
        msg = "policy validation must not iterate the configuration"
        raise AssertionError(msg)

    @typ.override
    def __len__(self) -> int:
        """Fail on any length check."""
        msg = "policy validation must not read the configuration length"
        raise AssertionError(msg)


@pytest.mark.parametrize(
    "policy", [0, 1, "false", None], ids=["zero", "one", "string", "none"]
)
def test_dict_config_rejects_non_bool_policy(policy: object) -> None:
    """A non-bool policy fails before the configuration is read."""
    config = _UnreadableConfig()
    with pytest.raises(TypeError, match="allow_factories must be a bool"):
        dictConfig(config, allow_factories=typ.cast("bool", policy))


@pytest.mark.parametrize(
    "factory_ref", [_FACTORY_PATH, ContextFilterFactory], ids=["dotted", "callable"]
)
@pytest.mark.parametrize("call_style", ["omitted", "explicit"], ids=["omitted", "true"])
def test_factory_mode_remains_default(factory_ref: object, call_style: str) -> None:
    """The default policy still resolves and invokes factories."""
    config = {
        "version": 1,
        "filters": {"factory": {"()": factory_ref, "request_id": "kept"}},
        "loggers": {"app": {"filters": ["factory"]}},
        "root": {"level": "DEBUG"},
    }
    if call_style == "omitted":
        dictConfig(config)
    else:
        dictConfig(config, allow_factories=True)

    collector = RecordCollector()
    get_logger("app").add_handler(collector)
    assert get_logger("app").log("INFO", "hello") is not None, (
        "the configured factory filter must accept the record"
    )
    wait_for(lambda: len(collector.records) == 1, "the record reaches the collector")
    record = collector.records[0]
    assert record["metadata"]["key_values"]["request_id"] == "kept", (
        "the factory keyword argument must still reach the record"
    )
