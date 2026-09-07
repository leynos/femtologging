"""Tests for filter sections handled by ``femtologging.dictConfig``.

Covers filter references from loggers and the root logger, the filtering
behaviour those references produce, and validation of malformed filter
definitions. Handler and general schema validation live in
``tests/test_dict_config.py``.
"""

from __future__ import annotations

import dataclasses

import pytest

from femtologging import dictConfig, get_logger, reset_manager


@pytest.mark.parametrize(
    ("config", "msg", "expected_exc"),
    [
        (
            {
                "version": 1,
                "loggers": {"app": {"filters": ["nonexistent"]}},
                "root": {"level": "DEBUG"},
            },
            r"nonexistent",
            KeyError,
        ),
        (
            {
                "version": 1,
                "root": {"level": "DEBUG", "filters": ["nonexistent"]},
            },
            r"nonexistent",
            KeyError,
        ),
        (
            {
                "version": 1,
                "filters": {"lvl": {"level": "INFO"}},
                "loggers": {"app": {"filters": "lvl"}},
                "root": {"level": "DEBUG"},
            },
            r"logger filters must be a list",
            TypeError,
        ),
        (
            {
                "version": 1,
                "filters": {"lvl": {"level": "INFO"}},
                "loggers": {"app": {"filters": ["lvl", 123]}},
                "root": {"level": "DEBUG"},
            },
            r"logger filters must be a list",
            TypeError,
        ),
        (
            {
                "version": 1,
                "filters": {"lvl": {"level": "INFO"}},
                "root": {"level": "DEBUG", "filters": [123]},
            },
            r"logger filters must be a list",
            TypeError,
        ),
    ],
    ids=[
        "logger-missing-filter-id",
        "root-missing-filter-id",
        "logger-filters-not-a-list",
        "logger-filters-non-string-items",
        "root-filters-non-string-items",
    ],
)
def test_dict_config_filters_errors(
    config: dict[str, object],
    msg: str,
    expected_exc: type[Exception],
) -> None:
    """Filter reference and type errors raise the expected exception."""
    reset_manager()
    with pytest.raises(expected_exc, match=msg):
        dictConfig(config)


@dataclasses.dataclass(frozen=True, slots=True)
class _FilteringCase:
    """One dictConfig filter setup plus the records it must allow and block."""

    config: dict[str, object]
    allowed_logger: str
    allowed_record: tuple[str, str]
    blocked_logger: str
    blocked_record: tuple[str, str]


@pytest.mark.parametrize(
    "case",
    [
        _FilteringCase(
            {
                "version": 1,
                "filters": {"lvl": {"level": "INFO"}},
                "loggers": {"app": {"filters": ["lvl"]}},
                "root": {"level": "DEBUG"},
            },
            "app",
            ("INFO", "allowed"),
            "app",
            ("ERROR", "suppressed"),
        ),
        _FilteringCase(
            {
                "version": 1,
                "filters": {"ns": {"name": "myapp"}},
                "loggers": {
                    "myapp": {"filters": ["ns"]},
                    "other": {"filters": ["ns"]},
                },
                "root": {"level": "DEBUG"},
            },
            "myapp",
            ("INFO", "pass"),
            "other",
            ("INFO", "blocked by name"),
        ),
        _FilteringCase(
            {
                "version": 1,
                "filters": {
                    "lvl": {"level": "INFO"},
                    "ns": {"name": "multi"},
                },
                "loggers": {"multi": {"filters": ["lvl", "ns"]}},
                "root": {"level": "DEBUG"},
            },
            "multi",
            ("INFO", "pass both"),
            "multi",
            ("ERROR", "blocked by level"),
        ),
        _FilteringCase(
            {
                "version": 1,
                "filters": {"lvl": {"level": "INFO"}},
                "root": {"level": "DEBUG", "filters": ["lvl"]},
            },
            "root",
            ("INFO", "allowed"),
            "root",
            ("ERROR", "blocked"),
        ),
    ],
    ids=[
        "level-filter",
        "name-filter",
        "multiple-filters",
        "root-filter",
    ],
)
def test_dict_config_filters_allow_and_block(case: _FilteringCase) -> None:
    """Configured filters should pass matching records and drop the rest."""
    reset_manager()
    dictConfig(case.config)

    allowed_level, allowed_message = case.allowed_record
    allowed = get_logger(case.allowed_logger).log(allowed_level, allowed_message)
    assert allowed is not None, (
        f"logger {case.allowed_logger!r} should emit "
        f"{allowed_level} {allowed_message!r}, got None"
    )

    blocked_level, blocked_message = case.blocked_record
    blocked = get_logger(case.blocked_logger).log(blocked_level, blocked_message)
    assert blocked is None, (
        f"logger {case.blocked_logger!r} should suppress "
        f"{blocked_level} {blocked_message!r}, got {blocked!r}"
    )


@pytest.mark.parametrize(
    ("filter_cfg", "msg", "expected_exc"),
    [
        (
            {"level": 42},
            r"filter 'f' level must be a string",
            TypeError,
        ),
        (
            {"name": 42},
            r"filter 'f' name must be a string",
            TypeError,
        ),
        (
            {"level": "INFO", "extra": True},
            r"filter 'f' has unsupported keys",
            ValueError,
        ),
        (
            {"level": "INFO", "name": "ns"},
            r"filter 'f' must contain 'level' or 'name', not both",
            ValueError,
        ),
    ],
    ids=[
        "level-type",
        "name-type",
        "unsupported-keys",
        "both-level-and-name",
    ],
)
def test_dict_config_filter_validation_errors(
    filter_cfg: dict[str, object],
    msg: str,
    expected_exc: type[Exception],
) -> None:
    """Malformed filter configurations raise the expected exception."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {"f": filter_cfg},
        "root": {},
    }
    with pytest.raises(expected_exc, match=msg):
        dictConfig(cfg)


def test_dict_config_filter_value_not_a_mapping() -> None:
    """A filter whose config value is not a mapping should raise."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {"f": "not-a-mapping"},
        "root": {},
    }
    with pytest.raises(TypeError, match="filter config must be a mapping"):
        dictConfig(cfg)


def test_dict_config_empty_filters_section() -> None:
    """An empty filters section should not cause errors."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {},
        "root": {"level": "DEBUG"},
    }
    dictConfig(cfg)
    emitted = get_logger("root").log("INFO", "emit")
    assert emitted is not None, (
        "an empty filters section must not suppress root records"
    )
