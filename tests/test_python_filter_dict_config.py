"""Tests for ``dictConfig`` factory-mode Python filter resolution."""

from __future__ import annotations

import sys
import types
import typing as typ

import pytest

from femtologging import dictConfig, get_logger
from tests.python_filter_support import (
    ContextFilterFactory,
    RecordCollector,
    wait_for,
)

_FACTORY_PATH = "tests.python_filter_support.ContextFilterFactory"
_NONCALLABLE_PATH = "tests.python_filter_support.NONCALLABLE_FACTORY"


class _RecordKeyValues(typ.TypedDict):
    """Typed record metadata read by the factory-mode filter assertions."""

    request_id: str


class _RecordMetadata(typ.TypedDict):
    """Typed metadata envelope emitted by the test record collector."""

    key_values: _RecordKeyValues


class _CollectedRecord(typ.TypedDict):
    """Narrow view of the record payload used in this module."""

    metadata: _RecordMetadata


def _collect_single_record(context: str) -> _CollectedRecord:
    """Attach a collector to ``app``, emit one record, and return it."""
    collector = RecordCollector()
    get_logger("app").add_handler(collector)

    assert get_logger("app").log("INFO", "hello") is not None, (
        f"{context}: the configured filter must accept the record"
    )
    wait_for(lambda: len(collector.records) == 1, "the record reaches the collector")
    return typ.cast("_CollectedRecord", collector.records[0])


def test_dict_config_filter_factory_form_supports_kwargs() -> None:
    """Factory-mode filter entries should resolve and instantiate callbacks."""
    dictConfig({
        "version": 1,
        "filters": {"factory": {"()": _FACTORY_PATH, "request_id": "factory-123"}},
        "loggers": {"app": {"filters": ["factory"]}},
        "root": {"level": "DEBUG"},
    })

    record = _collect_single_record("factory-mode filter")
    key_values = record["metadata"]["key_values"]
    assert key_values["request_id"] == "factory-123", (
        f"the factory keyword argument must reach the record, got {key_values!r}"
    )


def test_dict_config_filter_factory_rejects_unimportable_paths() -> None:
    """Factory-mode filters should surface import resolution failures."""
    cfg = {
        "version": 1,
        "filters": {"factory": {"()": "missing_python_filter_factory.factory"}},
        "root": {"level": "DEBUG"},
    }

    with pytest.raises(ValueError, match="failed to import filter factory"):
        dictConfig(cfg)


def test_dict_config_filter_factory_rejects_non_callable_objects() -> None:
    """Factory-mode filters should reject resolved non-callable objects."""
    cfg = {
        "version": 1,
        "filters": {"factory": {"()": _NONCALLABLE_PATH}},
        "root": {"level": "DEBUG"},
    }

    with pytest.raises(TypeError, match="factory must be callable"):
        dictConfig(cfg)


def test_dict_config_filter_factory_prefers_attributes_over_submodules() -> None:
    """Factory resolution should prefer attributes before importing submodules."""
    package = types.ModuleType("factory_pkg")
    package.__dict__["__path__"] = []
    package.__dict__["factory"] = ContextFilterFactory
    submodule = types.ModuleType("factory_pkg.factory")
    original_modules = dict(sys.modules)
    sys.modules["factory_pkg"] = package
    sys.modules["factory_pkg.factory"] = submodule

    try:
        dictConfig({
            "version": 1,
            "filters": {"factory": {"()": "factory_pkg.factory", "request_id": "attr"}},
            "loggers": {"app": {"filters": ["factory"]}},
            "root": {"level": "DEBUG"},
        })
    finally:
        sys.modules.clear()
        sys.modules.update(original_modules)

    record = _collect_single_record("attribute-resolved factory")
    key_values = record["metadata"]["key_values"]
    assert key_values["request_id"] == "attr", (
        "resolution must prefer the package attribute over the shadowing "
        f"submodule, got {key_values!r}"
    )


@pytest.mark.parametrize(
    "conflicting_key",
    [
        pytest.param("level", id="factory-plus-level"),
        pytest.param("name", id="factory-plus-name"),
    ],
)
def test_dict_config_rejects_mixed_factory_and_declarative_forms(
    conflicting_key: str,
) -> None:
    """Factory and declarative filter forms must remain mutually exclusive."""
    declarative_value = "INFO" if conflicting_key == "level" else "app"
    cfg = {
        "version": 1,
        "filters": {"f": {"()": _FACTORY_PATH, conflicting_key: declarative_value}},
        "root": {"level": "DEBUG"},
    }
    with pytest.raises(
        ValueError, match="must not mix '\\(\\)' with 'level' or 'name'"
    ):
        dictConfig(cfg)
