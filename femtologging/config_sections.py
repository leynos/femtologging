"""Configuration section processing for femtologging.dictConfig.

This module handles the processing of filters, formatters, handlers,
loggers, and root logger sections in ``dictConfig``-style configuration
dictionaries.

The entry points are :func:`_process_filters`, :func:`_process_formatters`,
:func:`_process_handlers`, :func:`_process_loggers`, and
:func:`_process_root_logger`, called from
:func:`femtologging.config.dictConfig`.
"""

from __future__ import annotations

import collections.abc as cabc
import typing as typ

from .config import (
    _build_filter_from_dict,
    _build_formatter,
    _build_handler_from_dict,
    _build_logger_from_dict,
    _validate_section_mapping,
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

    """
    mapping = _validate_section_mapping(config.get(section, {}), section)
    base_err_tmpl = key_err_tmpl or f"{item_name} ids must be strings"

    for key, cfg in mapping.items():
        if not isinstance(key, str):
            raise TypeError(base_err_tmpl.format(name=repr(key)))
        yield (
            key,
            typ.cast(
                "cabc.Mapping[str, object]",
                _validate_section_mapping(cfg, f"{item_name} config"),
            ),
        )


def _process_filters(
    builder: _ConfigBuilder, config: cabc.Mapping[str, object]
) -> _ConfigBuilder:
    """Attach filter builders to ``builder``."""
    for fid, filter_cfg in _iter_section_items(
        config,
        "filters",
        "filter",
    ):
        builder = builder.with_filter(fid, _build_filter_from_dict(fid, filter_cfg))
    return builder


def _process_formatters(
    builder: _ConfigBuilder, config: cabc.Mapping[str, object]
) -> _ConfigBuilder:
    """Attach formatter builders to ``builder``."""
    for fid, fmt_cfg in _iter_section_items(
        config,
        "formatters",
        "formatter",
    ):
        builder = builder.with_formatter(fid, _build_formatter(fmt_cfg))
    return builder


def _process_handlers(
    builder: _ConfigBuilder, config: cabc.Mapping[str, object]
) -> _ConfigBuilder:
    """Attach handler builders to ``builder``."""
    for hid, handler_cfg in _iter_section_items(
        config,
        "handlers",
        "handler",
    ):
        builder = builder.with_handler(hid, _build_handler_from_dict(hid, handler_cfg))
    return builder


def _process_loggers(
    builder: _ConfigBuilder, config: cabc.Mapping[str, object]
) -> _ConfigBuilder:
    """Attach logger configurations to ``builder``."""
    for lname, logger_cfg in _iter_section_items(
        config,
        "loggers",
        "logger",
        key_err_tmpl="loggers section key {name} must be a string",
    ):
        builder = builder.with_logger(lname, _build_logger_from_dict(lname, logger_cfg))
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
