"""Configuration section processing for femtologging.dictConfig.

This module handles the processing of filters, formatters, handlers,
loggers, and root logger sections in ``dictConfig``-style configuration
dictionaries.

The entry points are :func:`_reject_filter_factories`,
:func:`_process_sections`, and :func:`_process_root_logger`, called from
:func:`femtologging.config.dictConfig`.
"""

from __future__ import annotations

import collections.abc as cabc
import dataclasses
import typing as typ

from ._config_filters import build_filter_from_dict
from ._config_policy import factory_disallowed_message
from .config import (
    _build_formatter,
    _build_handler_from_dict,
    _build_logger_from_dict,
    _validate_mapping_type,
    _validate_string_keys,
)

if typ.TYPE_CHECKING:
    from .config_protocol import _ConfigBuilder


def _iter_section_items(
    config: cabc.Mapping[str, object],
    section: str,
    item_name: str,
    *,
    key_err_tmpl: str | None = None,
) -> cabc.Iterator[tuple[str, cabc.Mapping[str, object]]]:
    """Iterate over validated section items.

    Parameters
    ----------
    config
        The full dictConfig mapping.
    section
        The section name (e.g., "formatters", "handlers", "loggers").
    item_name
        Singular item name used in error messages.
    key_err_tmpl
        Optional error template for non-string keys. May contain {name} placeholder.

    Yields
    ------
    tuple[str, cabc.Mapping[str, object]]
        (id, config) pairs for each item in the section.

    Raises
    ------
    TypeError
        If a section key is not a string.

    """
    mapping = _validate_mapping_type(config.get(section, {}), section)
    base_err_tmpl = key_err_tmpl or f"{item_name} ids must be strings"

    for key, cfg in mapping.items():
        if not isinstance(key, str):
            raise TypeError(base_err_tmpl.format(name=repr(key)))
        yield (
            key,
            _validate_string_keys(
                _validate_mapping_type(cfg, f"{item_name} config"),
                f"{item_name} config",
            ),
        )


@dataclasses.dataclass(frozen=True, slots=True)
class _SectionSpec:
    """Describe a section whose entries each build one configured object."""

    section: str
    item_name: str
    attach: cabc.Callable[
        [_ConfigBuilder, str, cabc.Mapping[str, object]], _ConfigBuilder
    ]
    key_err_tmpl: str | None = None


_SECTION_SPECS: tuple[_SectionSpec, ...] = (
    _SectionSpec(
        "formatters",
        "formatter",
        lambda builder, fid, cfg: builder.with_formatter(fid, _build_formatter(cfg)),
    ),
    _SectionSpec(
        "handlers",
        "handler",
        lambda builder, hid, cfg: builder.with_handler(
            hid, _build_handler_from_dict(hid, cfg)
        ),
    ),
    _SectionSpec(
        "loggers",
        "logger",
        lambda builder, lname, cfg: builder.with_logger(
            lname, _build_logger_from_dict(lname, cfg)
        ),
        key_err_tmpl="loggers section key {name} must be a string",
    ),
)


def _attach_section(
    builder: _ConfigBuilder,
    config: cabc.Mapping[str, object],
    spec: _SectionSpec,
) -> _ConfigBuilder:
    """Attach every validated item in ``spec``'s section."""
    for item_id, item_cfg in _iter_section_items(
        config, spec.section, spec.item_name, key_err_tmpl=spec.key_err_tmpl
    ):
        builder = spec.attach(builder, item_id, item_cfg)
    return builder


def _reject_filter_factories(config: cabc.Mapping[str, object]) -> None:
    """Reject every ``"()"`` filter entry before any builder is created.

    This whole-section preflight runs before ``ConfigBuilder`` construction, so
    declarative-only mode refuses factory entries before their target can be
    imported, resolved, or invoked. Detection is by key presence alone: the
    value is never inspected. A mixed entry carrying ``"()"`` alongside
    ``level`` or ``name`` is rejected with the same error, ahead of the
    mixed-key validation that would otherwise report it differently.

    Parameters
    ----------
    config
        The full ``dictConfig`` mapping. Section structure is validated while
        iterating, so malformed filter sections still raise their usual
        structural errors.

    Raises
    ------
    ValueError
        If any filter entry contains a ``"()"`` key.

    """
    for fid, filter_cfg in _iter_section_items(config, "filters", "filter"):
        if "()" in filter_cfg:
            msg = factory_disallowed_message(fid)
            raise ValueError(msg)


def _process_filters(
    builder: _ConfigBuilder,
    config: cabc.Mapping[str, object],
    *,
    allow_factories: bool = True,
) -> _ConfigBuilder:
    """Attach filter builders to ``builder``.

    Passing ``allow_factories=False`` keeps the per-entry guard active as well
    as the caller's whole-section preflight. That guard protects direct callers
    and any mapping that changes between the preflight and this pass.

    Returns
    -------
    _ConfigBuilder
        ``builder`` with every filter entry attached.
    """
    for fid, filter_cfg in _iter_section_items(
        config,
        "filters",
        "filter",
    ):
        builder = builder.with_filter(
            fid,
            build_filter_from_dict(
                fid, dict(filter_cfg), allow_factories=allow_factories
            ),
        )
    return builder


def _process_sections(
    builder: _ConfigBuilder,
    config: cabc.Mapping[str, object],
    *,
    allow_factories: bool = True,
) -> _ConfigBuilder:
    """Attach the filter, formatter, handler, and logger sections."""
    builder = _process_filters(builder, config, allow_factories=allow_factories)
    for spec in _SECTION_SPECS:
        builder = _attach_section(builder, config, spec)
    return builder


def _process_root_logger(
    builder: _ConfigBuilder, config: cabc.Mapping[str, object]
) -> _ConfigBuilder:
    """Configure the root logger."""
    if "root" not in config:
        msg = "root logger configuration is required"
        raise ValueError(msg)
    root = config["root"]
    if not isinstance(root, cabc.Mapping):
        msg = "root logger configuration must be a mapping"
        raise TypeError(msg)
    return builder.with_root_logger(
        _build_logger_from_dict("root", typ.cast("cabc.Mapping[str, object]", root))
    )
