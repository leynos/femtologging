"""Tests for declarative-only ``dictConfig`` filter handling.

``allow_factories=False`` rejects filter factories only; it applies no trust
policy to handler classes, destinations, or the rest of the document. These
tests prove that the mode accepts built-in ``level`` and ``name`` filters,
rejects every ``"()"`` form before import, resolution, invocation, or
construction, and leaves default factory behaviour unchanged.
"""

from __future__ import annotations

import collections.abc as cabc
import dataclasses
import logging
import sys
import typing as typ
import uuid

import pytest

import femtologging._config_filters as config_filters
import femtologging.config as config_module
from femtologging import basicConfig, dictConfig, get_logger
from femtologging._config_filters import build_filter_from_dict
from tests.python_filter_support import (
    ContextFilterFactory,
    RecordCollector,
    wait_for,
)

if typ.TYPE_CHECKING:
    from pathlib import Path

_FACTORY_PATH = "tests.python_filter_support.ContextFilterFactory"
_POLICY_PATTERN = r"allow_factories=False"
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


def _plain_factory(**_kwargs: object) -> object:
    """Return a filter callable without observing any side effect."""
    return lambda _record: True


class _PlainFactoryClass:
    """Class target that would accept the factory keyword arguments."""

    def __init__(self, **_kwargs: object) -> None:
        """Accept keyword arguments without observing any side effect."""


_FACTORY_VALUE_BUILDERS: tuple[
    tuple[str, cabc.Callable[[_FactoryProbe], object]], ...
] = (
    ("unimported-dotted", lambda _probe: "declarative_guard_missing.missing"),
    ("importable-dotted", lambda _probe: _FACTORY_PATH),
    ("module-dotted", lambda _probe: "tests.python_filter_support"),
    ("function", lambda _probe: _plain_factory),
    ("class", lambda _probe: _PlainFactoryClass),
    ("filter-class", lambda _probe: logging.Filter),
    ("callable-instance", lambda probe: probe),
    ("lambda", lambda _probe: lambda **_kwargs: None),
    ("none", lambda _probe: None),
    ("zero", lambda _probe: 0),
    ("false", lambda _probe: False),
    ("empty-string", lambda _probe: ""),
    ("list", lambda _probe: []),
    ("dict", lambda _probe: {}),
)


_FACTORY_ENTRY_BUILDERS: tuple[
    tuple[str, cabc.Callable[[_FactoryProbe], dict[str, object]]], ...
] = (
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
    ],
    ids=["logger-level-filter", "logger-name-filter", "root-level-filter"],
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


@pytest.mark.parametrize(
    "value_builder",
    [builder for _, builder in _FACTORY_VALUE_BUILDERS],
    ids=[name for name, _ in _FACTORY_VALUE_BUILDERS],
)
@pytest.mark.parametrize(
    "extra_keys", [{}, {"request_id": "svc"}], ids=["bare", "kwargs"]
)
def test_declarative_mode_rejects_every_factory_value(
    monkeypatch: pytest.MonkeyPatch,
    value_builder: cabc.Callable[[_FactoryProbe], object],
    extra_keys: dict[str, object],
) -> None:
    """Every ``"()"`` value is rejected without resolution or invocation."""
    probe = _FactoryProbe()
    monkeypatch.setattr(
        config_filters, "resolve_factory", _unexpected_call("resolve_factory")
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
    config = {
        "version": 1,
        "filters": {"lvl": {"level": "INFO"}, "f": {"()": _FACTORY_PATH}},
        "loggers": {"app": {"filters": ["lvl"]}},
        "root": {"level": "DEBUG"},
    }
    app = get_logger("app")
    root = get_logger("root")
    before = (app.level, app.disabled, root.level, root.disabled)
    with pytest.raises(ValueError, match=_FILTER_ERROR_PATTERN):
        dictConfig(config, allow_factories=False)

    after = (app.level, app.disabled, root.level, root.disabled)
    assert after == before, "rejection must leave both loggers' state unchanged"
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
    entry_builder: cabc.Callable[[_FactoryProbe], dict[str, object]],
) -> None:
    """The per-entry guard rejects dotted, callable, and mixed entries."""
    probe = _FactoryProbe()
    monkeypatch.setattr(
        config_filters, "resolve_factory", _unexpected_call("resolve_factory")
    )

    with pytest.raises(ValueError, match=_POLICY_PATTERN):
        build_filter_from_dict("f", entry_builder(probe), allow_factories=False)
    assert not probe.invocations, "the direct guard must not invoke callables"


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
