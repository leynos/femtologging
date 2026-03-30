"""Unit tests for ``femtologging._filter_factory``."""

from __future__ import annotations

import importlib
import re
from types import SimpleNamespace

import pytest

from femtologging import _filter_factory as filter_factory


def test_try_import_module_returns_none_for_missing_root_module(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Missing root-module imports should resolve to ``None``."""

    def fake_import(name: str) -> object:
        raise ModuleNotFoundError(name=name)

    monkeypatch.setattr(importlib, "import_module", fake_import)

    assert filter_factory._try_import_module("missing_package") is None


def test_try_import_module_reraises_nested_import_failures(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Nested import failures should not be mistaken for a missing root path."""

    def fake_import(name: str) -> object:
        raise ModuleNotFoundError(name="nested_dependency")

    monkeypatch.setattr(importlib, "import_module", fake_import)

    with pytest.raises(ModuleNotFoundError) as exc_info:
        filter_factory._try_import_module("pkg")
    assert exc_info.value.name == "nested_dependency"


@pytest.mark.parametrize(
    ("dotted_path", "modules", "expected"),
    [
        (
            "pkg.Factory",
            {"pkg": SimpleNamespace(Factory="factory-object")},
            "factory-object",
        ),
        (
            "pkg.sub.Factory",
            {
                "pkg": SimpleNamespace(),
                "pkg.sub": SimpleNamespace(Factory="nested-factory"),
            },
            "nested-factory",
        ),
        (
            "pkg.Container.method",
            {
                "pkg": SimpleNamespace(
                    Container=SimpleNamespace(method="container-method")
                )
            },
            "container-method",
        ),
    ],
    ids=["root_attr", "submodule_attr", "nested_attr"],
)
def test_resolve_factory_resolves_supported_dotted_paths(
    monkeypatch: pytest.MonkeyPatch,
    dotted_path: str,
    modules: dict[str, object],
    expected: object,
) -> None:
    """Factory resolution should follow stdlib-style module and attribute hops."""

    def fake_import(name: str) -> object:
        try:
            return modules[name]
        except KeyError as exc:
            raise ModuleNotFoundError(name=name) from exc

    monkeypatch.setattr(importlib, "import_module", fake_import)

    assert filter_factory.resolve_factory(dotted_path) == expected


@pytest.mark.parametrize(
    ("dotted_path", "message"),
    [
        ("factory", "invalid filter factory path 'factory'"),
        ("missing.Factory", "failed to import filter factory 'missing.Factory'"),
        (
            "pkg.missing.Factory",
            "failed to resolve filter factory 'pkg.missing.Factory'",
        ),
    ],
    ids=["invalid", "missing_root", "missing_nested"],
)
def test_resolve_factory_rejects_invalid_paths(
    monkeypatch: pytest.MonkeyPatch,
    dotted_path: str,
    message: str,
) -> None:
    """Invalid dotted paths should raise consistent ``ValueError`` messages."""
    monkeypatch.setattr(
        importlib,
        "import_module",
        lambda name: (
            SimpleNamespace()
            if name == "pkg"
            else (_ for _ in ()).throw(ModuleNotFoundError(name=name))
        ),
    )

    with pytest.raises(ValueError, match=re.escape(message)):
        filter_factory.resolve_factory(dotted_path)
