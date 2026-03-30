"""Unit tests for ``femtologging._config_filters``."""

from __future__ import annotations

import re
import typing as typ

import pytest

from femtologging import _config_filters as config_filters


class FakeLevelFilterBuilder:
    """Record the level passed to ``with_max_level``."""

    def __init__(self) -> None:
        """Initialize the captured level."""
        self.level: str | None = None

    def with_max_level(self, level: str) -> tuple[str, str]:
        """Return a simple marker for assertions."""
        self.level = level
        return ("level", level)


class FakeNameFilterBuilder:
    """Record the name passed to ``with_prefix``."""

    def __init__(self) -> None:
        """Initialize the captured prefix."""
        self.prefix: str | None = None

    def with_prefix(self, prefix: str) -> tuple[str, str]:
        """Return a simple marker for assertions."""
        self.prefix = prefix
        return ("name", prefix)


class FakePythonCallbackFilterBuilder:
    """Capture the built callback filter object."""

    def __init__(self, callback: object) -> None:
        """Store the callback for later assertions."""
        self.callback = callback


@pytest.mark.parametrize(
    ("data", "message"),
    [
        ({}, "filter 'f' must contain a 'level', 'name', or '()' key"),
        (
            {"level": "INFO", "name": "app"},
            "filter 'f' must contain 'level' or 'name', not both",
        ),
        (
            {"level": "INFO", "extra": True},
            "filter 'f' has unsupported keys: ['extra']",
        ),
        (
            {"()": "pkg.factory", "level": "INFO"},
            "filter 'f' must not mix '()' with 'level' or 'name'",
        ),
    ],
    ids=["missing", "both_declarative", "unsupported", "mixed_factory"],
)
def test_validate_filter_config_keys_rejects_invalid_shapes(
    data: dict[str, object], message: str
) -> None:
    """Validation should reject malformed declarative and factory forms."""
    with pytest.raises(ValueError, match=re.escape(message)):
        config_filters.validate_filter_config_keys("f", data)


@pytest.mark.parametrize(
    ("data", "expected"),
    [
        ({"level": "INFO"}, ("level", "INFO")),
        ({"name": "svc"}, ("name", "svc")),
    ],
    ids=["level", "name"],
)
def test_build_filter_from_dict_builds_declarative_filters(
    monkeypatch: pytest.MonkeyPatch,
    data: dict[str, object],
    expected: tuple[str, str],
) -> None:
    """Declarative entries should map to the matching Rust builder surface."""
    monkeypatch.setattr(config_filters, "LevelFilterBuilder", FakeLevelFilterBuilder)
    monkeypatch.setattr(config_filters, "NameFilterBuilder", FakeNameFilterBuilder)

    assert config_filters.build_filter_from_dict("f", data) == expected


def test_build_filter_from_dict_resolves_factory_paths(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """String factories should be resolved and wrapped as callback filters."""
    built_with: dict[str, object] = {}

    def factory(**kwargs: object) -> object:
        built_with.update(kwargs)
        return {"built": kwargs}

    monkeypatch.setattr(config_filters, "resolve_factory", lambda dotted: factory)
    monkeypatch.setattr(
        config_filters,
        "PythonCallbackFilterBuilder",
        FakePythonCallbackFilterBuilder,
    )

    result = typ.cast(
        "FakePythonCallbackFilterBuilder",
        config_filters.build_filter_from_dict(
            "f",
            {"()": "pkg.filter_factory", "request_id": "req-123"},
        ),
    )

    assert built_with == {"request_id": "req-123"}
    assert result.callback == {"built": {"request_id": "req-123"}}


def test_build_filter_from_dict_accepts_direct_callable_factories(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Direct callable factories should bypass dotted-path resolution."""
    monkeypatch.setattr(
        config_filters,
        "PythonCallbackFilterBuilder",
        FakePythonCallbackFilterBuilder,
    )

    def factory(*, enabled: bool) -> dict[str, bool]:
        return {"enabled": enabled}

    result = typ.cast(
        "FakePythonCallbackFilterBuilder",
        config_filters.build_filter_from_dict(
            "f",
            {"()": factory, "enabled": True},
        ),
    )

    assert result.callback == {"enabled": True}


def test_build_filter_from_dict_rejects_non_callable_factories(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Factory resolution should fail fast when the target is not callable."""
    monkeypatch.setattr(config_filters, "resolve_factory", lambda dotted: object())

    with pytest.raises(
        TypeError, match=re.escape("filter 'f' factory must be callable")
    ):
        config_filters.build_filter_from_dict("f", {"()": "pkg.invalid"})
