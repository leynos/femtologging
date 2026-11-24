"""Tests for factory rejection under declarative-only ``dictConfig``.

``allow_factories=False`` rejects every ``"()"`` filter entry before it can be
imported, resolved, invoked, or turned into a builder. The tests use
independent sentinels at each seam, so a future refactor cannot quietly move
the policy check after a side effect.
"""

from __future__ import annotations

import collections.abc as cabc
import importlib
import logging
import sys
import typing as typ
import uuid

import pytest

import femtologging._config_filters as config_filters
import femtologging.config as config_module
from femtologging import basicConfig, dictConfig, get_logger
from femtologging._config_filters import build_filter_from_dict

if typ.TYPE_CHECKING:
    from pathlib import Path

_FACTORY_PATH = "tests.python_filter_support.ContextFilterFactory"
_FILTER_ERROR_PATTERN = r"filter 'f'.*allow_factories=False"


class _FactoryProbe:
    """Record the side effects a rejected factory could have produced."""

    def __init__(self) -> None:
        """Start with no observed invocations."""
        self.invocations: list[dict[str, object]] = []

    def __call__(self, **kwargs: object) -> object:
        """Record the keyword arguments and return an accepting filter."""
        self.invocations.append(kwargs)
        return lambda _record: True


type _ValueBuilder = cabc.Callable[[_FactoryProbe], object]
type _EntryBuilder = cabc.Callable[[_FactoryProbe], cabc.Mapping[str, object]]


def _counting_function(probe: _FactoryProbe) -> cabc.Callable[..., object]:
    """Return a function that records any invocation on ``probe``."""

    def _factory(**kwargs: object) -> object:
        """Record the call and return an accepting filter."""
        probe.invocations.append(kwargs)
        return lambda _record: True

    return _factory


def _counting_class(probe: _FactoryProbe) -> type:
    """Return a class whose construction records the invocation."""

    class _CountingFactory:
        """Record any construction attempt as a factory invocation."""

        def __init__(self, **kwargs: object) -> None:
            """Record the keyword arguments construction received."""
            probe.invocations.append(kwargs)

    return _CountingFactory


def _counting_filter_class(probe: _FactoryProbe) -> type:
    """Return a ``logging.Filter`` subclass that records construction."""

    class _CountingFilter(logging.Filter):
        """Record any construction attempt as a factory invocation."""

        def __init__(self, **kwargs: object) -> None:
            """Record the keyword arguments construction received."""
            super().__init__()
            probe.invocations.append(kwargs)

    return _CountingFilter


def _counting_lambda(probe: _FactoryProbe) -> cabc.Callable[..., object]:
    """Return a lambda that records any invocation on ``probe``."""
    return lambda **kwargs: probe.invocations.append(kwargs)


_FACTORY_VALUE_BUILDERS: tuple[tuple[str, _ValueBuilder], ...] = (
    ("unimported-dotted", lambda _probe: "declarative_guard_missing.missing"),
    ("importable-dotted", lambda _probe: _FACTORY_PATH),
    ("module-dotted", lambda _probe: "tests.python_filter_support"),
    ("function", _counting_function),
    ("class", _counting_class),
    ("filter-class", _counting_filter_class),
    ("callable-instance", lambda probe: probe),
    ("lambda", _counting_lambda),
    ("none", lambda _probe: None),
    ("zero", lambda _probe: 0),
    ("false", lambda _probe: False),
    ("empty-string", lambda _probe: ""),
    ("list", lambda _probe: []),
    ("dict", lambda _probe: {}),
)


_FACTORY_ENTRY_BUILDERS: tuple[tuple[str, _EntryBuilder], ...] = (
    ("dotted", lambda _probe: {"()": _FACTORY_PATH}),
    ("callable", lambda probe: {"()": probe}),
    ("mixed", lambda _probe: {"()": _FACTORY_PATH, "level": "INFO"}),
)


def _factory_config(
    factory_ref: object, extra_keys: dict[str, object]
) -> dict[str, object]:
    """Return a minimal ``dictConfig`` document with one factory entry."""
    return {
        "version": 1,
        "filters": {"f": {"()": factory_ref, **extra_keys}},
        "root": {"level": "DEBUG"},
    }


def _unexpected_call(name: str) -> cabc.Callable[..., object]:
    """Return a sentinel that fails when a forbidden call happens."""

    def _fail(*_args: object, **_kwargs: object) -> object:
        """Fail the test if the guarded seam is reached."""
        msg = f"{name} must not be called for a rejected factory"
        raise AssertionError(msg)

    return _fail


@pytest.mark.parametrize(
    "value_builder",
    [builder for _, builder in _FACTORY_VALUE_BUILDERS],
    ids=[name for name, _ in _FACTORY_VALUE_BUILDERS],
)
@pytest.mark.parametrize(
    "extra_keys",
    [{}, {"request_id": "svc"}, {"level": "INFO"}, {"name": "svc"}],
    ids=["bare", "kwargs", "mixed-level", "mixed-name"],
)
def test_declarative_mode_rejects_every_factory_value(
    monkeypatch: pytest.MonkeyPatch,
    value_builder: _ValueBuilder,
    extra_keys: dict[str, object],
) -> None:
    """Every ``"()"`` value fails before import, resolution, or invocation."""
    probe = _FactoryProbe()
    monkeypatch.setattr(
        config_filters, "resolve_factory", _unexpected_call("resolve_factory")
    )
    monkeypatch.setattr(
        importlib, "import_module", _unexpected_call("importlib.import_module")
    )

    config = _factory_config(value_builder(probe), extra_keys)
    with pytest.raises(ValueError, match=_FILTER_ERROR_PATTERN):
        dictConfig(config, allow_factories=False)

    assert not probe.invocations, "a rejected factory target must not be invoked"


@pytest.mark.parametrize(
    ("declarative_key", "declarative_value"),
    [("level", "INFO"), ("name", "svc")],
)
def test_declarative_mode_rejects_mixed_factory_entries(
    declarative_key: str, declarative_value: str
) -> None:
    """The factory rejection wins over mixed-key validation in safe mode."""
    config = {
        "version": 1,
        "filters": {"f": {"()": _FACTORY_PATH, declarative_key: declarative_value}},
        "root": {"level": "DEBUG"},
    }
    with pytest.raises(ValueError, match=_FILTER_ERROR_PATTERN):
        dictConfig(config, allow_factories=False)


def test_declarative_mode_rejects_before_importing_factory_module(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A rejected dotted factory must not import the module that holds it."""
    marker = tmp_path / "imported.marker"
    # A unique name keeps the cache assertions robust if a plugin or another
    # test has already loaded a module with a similar name.
    module_name = f"declarative_guard_{uuid.uuid4().hex}"
    module_path = tmp_path / f"{module_name}.py"
    module_path.write_text(
        "from pathlib import Path\n"
        f"Path({str(marker)!r}).write_text('imported')\n"
        "def build(**_kwargs: object) -> object:\n"
        "    return lambda _record: True\n"
    )
    monkeypatch.syspath_prepend(str(tmp_path))
    try:
        config = _factory_config(f"{module_name}.build", {})
        with pytest.raises(ValueError, match=_FILTER_ERROR_PATTERN):
            dictConfig(config, allow_factories=False)
        assert not marker.exists(), "the factory module must not be imported"
        assert module_name not in sys.modules, (
            "the factory module must not be left in sys.modules"
        )
    finally:
        # Only this test's temporary module belongs to it; modules imported by
        # unrelated code while it runs must stay in the process-wide cache.
        sys.modules.pop(module_name, None)


def test_declarative_mode_rejects_before_builder_construction(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A valid entry before the rejected factory must build nothing either."""
    constructed: list[object] = []

    def _fail_construction(*args: object, **_kwargs: object) -> object:
        """Record and fail any attempt to build a filter or builder."""
        constructed.append(args)
        msg = "no builder may be constructed for a rejected config"
        raise AssertionError(msg)

    monkeypatch.setattr(config_module, "_create_config_builder", _fail_construction)
    monkeypatch.setattr(config_filters, "_build_declarative_filter", _fail_construction)
    config = {
        "version": 1,
        "filters": {"ok": {"level": "INFO"}, "f": {"()": _FACTORY_PATH}},
        "root": {"level": "DEBUG"},
    }
    with pytest.raises(ValueError, match=_FILTER_ERROR_PATTERN):
        dictConfig(config, allow_factories=False)
    assert not constructed, "rejection must precede any builder construction"


def test_declarative_mode_rejection_leaves_live_state_unchanged() -> None:
    """A rejected document must not apply any of its logger configuration."""
    basicConfig(level="ERROR")
    app = get_logger("declarative_rejection_probe")
    app.set_level("DEBUG")
    config = {
        "version": 1,
        "filters": {"lvl": {"level": "INFO"}, "f": {"()": _FACTORY_PATH}},
        "loggers": {"declarative_rejection_probe": {"filters": ["lvl"]}},
        "root": {"level": "DEBUG"},
    }
    root = get_logger("root")
    assert app.log("ERROR", "before") is not None, "probe must start unfiltered"
    assert root.log("ERROR", "before") is not None, "root must start permissive"
    with pytest.raises(ValueError, match=_FILTER_ERROR_PATTERN):
        dictConfig(config, allow_factories=False)

    assert app.log("ERROR", "after") is not None, (
        "rejection must not attach the staged INFO-only filter"
    )
    assert root.log("ERROR", "after") is not None, (
        "rejection must not raise the pre-existing root level"
    )
    assert root.log("INFO", "still suppressed") is None, (
        "rejection must leave the pre-existing root level in place"
    )


@pytest.mark.parametrize(
    "entry_builder",
    [builder for _, builder in _FACTORY_ENTRY_BUILDERS],
    ids=[name for name, _ in _FACTORY_ENTRY_BUILDERS],
)
def test_build_filter_from_dict_rejects_factory_forms(
    monkeypatch: pytest.MonkeyPatch,
    entry_builder: _EntryBuilder,
) -> None:
    """The per-entry guard rejects dotted, callable, and mixed entries."""
    probe = _FactoryProbe()
    monkeypatch.setattr(
        config_filters, "resolve_factory", _unexpected_call("resolve_factory")
    )
    monkeypatch.setattr(
        importlib, "import_module", _unexpected_call("importlib.import_module")
    )

    with pytest.raises(ValueError, match=_FILTER_ERROR_PATTERN):
        build_filter_from_dict("f", dict(entry_builder(probe)), allow_factories=False)
    assert not probe.invocations, "the direct guard must not invoke callables"
