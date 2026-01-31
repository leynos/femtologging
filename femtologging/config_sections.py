"""Configuration section processing for femtologging.dictConfig.

This module handles the processing of formatters, handlers, loggers,
and root logger sections in ``dictConfig``-style configuration dictionaries.

The entry points are :func:`_process_formatters`, :func:`_process_handlers`,
:func:`_process_loggers`, and :func:`_process_root_logger`, called from
:func:`femtologging.config.dictConfig`.
"""

from __future__ import annotations

import collections.abc as cabc
import dataclasses
import typing as typ

from .config import (
    _build_formatter,
    _build_handler_from_dict,
    _build_logger_from_dict,
    _validate_section_mapping,
)

Mapping = cabc.Mapping
Callable = cabc.Callable
Any = typ.Any
cast = typ.cast


@dataclasses.dataclass(frozen=True)
class SectionProcessor:
    """Configuration for :func:`_process_config_section`."""

    section: str
    builder_method: str
    build_func: Callable[[str, Mapping[str, object]], object]
    err_tmpl: str | None = None


def _process_config_section(
    builder: Any, config: Mapping[str, object], processor: SectionProcessor
) -> None:
    """Process formatter, handler, and logger sections."""
    mapping = cast(
        "Mapping[object, object]",
        _validate_section_mapping(config.get(processor.section, {}), processor.section),
    )
    method = getattr(builder, processor.builder_method)
    for key, cfg in mapping.items():
        if not isinstance(key, str):
            if processor.err_tmpl is None:
                msg = f"{processor.section[:-1]} ids must be strings"
                raise TypeError(msg)
            raise TypeError(processor.err_tmpl.format(name=repr(key)))
        method(
            key,
            processor.build_func(
                key,
                _validate_section_mapping(cfg, f"{processor.section[:-1]} config"),
            ),
        )


def _process_formatters(builder: Any, config: Mapping[str, object]) -> None:
    """Attach formatter builders to ``builder``."""
    _process_config_section(
        builder,
        config,
        SectionProcessor(
            "formatters", "with_formatter", lambda fid, m: _build_formatter(m)
        ),
    )


def _process_handlers(builder: Any, config: Mapping[str, object]) -> None:
    """Attach handler builders to ``builder``."""
    _process_config_section(
        builder,
        config,
        SectionProcessor("handlers", "with_handler", _build_handler_from_dict),
    )


def _process_loggers(builder: Any, config: Mapping[str, object]) -> None:
    """Attach logger configurations to ``builder``."""
    _process_config_section(
        builder,
        config,
        SectionProcessor(
            "loggers",
            "with_logger",
            _build_logger_from_dict,
            err_tmpl="loggers section key {name} must be a string",
        ),
    )


def _process_root_logger(builder: Any, config: Mapping[str, object]) -> None:
    """Configure the root logger."""
    if "root" not in config:
        msg = "root logger configuration is required"
        raise ValueError(msg)
    root = config["root"]
    if not isinstance(root, Mapping):
        msg = "root logger configuration must be a mapping"
        raise TypeError(msg)
    builder.with_root_logger(
        _build_logger_from_dict("root", cast("Mapping[str, object]", root))
    )
