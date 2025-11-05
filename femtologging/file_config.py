"""Load logging configuration from INI files.

This module implements :func:`fileConfig`, a compatibility layer mirroring the
behaviour of :func:`logging.config.fileConfig`. INI parsing is delegated to the
Rust extension so we benefit from the same parser on every platform. The
resulting structure is translated into a ``dictConfig`` dictionary, ensuring the
builder API remains the canonical configuration mechanism.
"""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path
import re
from typing import Any

from . import _femtologging_rs as rust
from .config import dictConfig

_DEFAULT_SECTION = "DEFAULT"
_PERCENT_PLACEHOLDER = re.compile(r"%\(([^)]+)\)s")


def fileConfig(
    fname: str | bytes | Path,
    defaults: Mapping[str, object] | None = None,
    *,
    disable_existing_loggers: bool = True,
    encoding: str | None = None,
) -> None:
    """Configure logging using an INI file.

    Parameters mirror :func:`logging.config.fileConfig`, but the parsed data is
    converted into :func:`dictConfig` structures, preserving femtologging's
    builder-first design.

    Examples
    --------
    >>> fileConfig("tests/data/basic_file_config.ini")
    """

    sections = rust.parse_ini_file(str(Path(fname)), encoding)
    config = _ini_to_dict_config(sections, defaults, bool(disable_existing_loggers))
    dictConfig(config)


def _ini_to_dict_config(
    sections: list[tuple[str, list[tuple[str, str]]]],
    defaults: Mapping[str, object] | None,
    disable_existing: bool,
) -> dict[str, Any]:
    section_map = _materialise_sections(sections)
    _reject_formatters(section_map)
    default_pool = _merge_defaults(section_map.pop(_DEFAULT_SECTION, {}), defaults)
    formatters = _parse_formatters(section_map)
    handlers = _parse_handlers(section_map, default_pool)
    loggers, root = _parse_loggers(section_map)

    cfg: dict[str, Any] = {
        "version": 1,
        "disable_existing_loggers": disable_existing,
        "handlers": handlers,
        "root": root,
    }
    if formatters:
        cfg["formatters"] = formatters
    if loggers:
        cfg["loggers"] = loggers
    return cfg


def _materialise_sections(
    sections: list[tuple[str, list[tuple[str, str]]]],
) -> dict[str, dict[str, str]]:
    result: dict[str, dict[str, str]] = {}
    for name, entries in sections:
        mapping = result.setdefault(name, {})
        for key, value in entries:
            mapping[key] = value
    return result


def _reject_formatters(sections: dict[str, dict[str, str]]) -> None:
    fmt_section = sections.pop("formatters", None)
    if not fmt_section:
        return
    if _split_csv(fmt_section.get("keys")):
        raise ValueError("formatters are not supported")


def _merge_defaults(
    ini_defaults: Mapping[str, str],
    user_defaults: Mapping[str, object] | None,
) -> dict[str, str]:
    merged: dict[str, str] = {}
    if user_defaults:
        for key, value in user_defaults.items():
            merged[str(key)] = str(value)
    merged.update(ini_defaults)
    return merged


def _parse_formatters(
    sections: dict[str, dict[str, str]],
) -> dict[str, dict[str, str]]:
    fmt_section = sections.get("formatters")
    if not fmt_section:
        return {}
    formatter_ids = _split_csv(fmt_section.get("keys"))
    formatters: dict[str, dict[str, str]] = {}
    for fid in formatter_ids:
        section = _require_section(sections, f"formatter_{fid}")
        unknown = set(section) - {"format", "datefmt"}
        if unknown:
            raise ValueError(
                f"formatter {fid!r} has unsupported options: {sorted(unknown)!r}"
            )
        config: dict[str, str] = {}
        if section.get("format") is not None:
            config["format"] = section["format"]  # type: ignore[index]
        if section.get("datefmt") is not None:
            config["datefmt"] = section["datefmt"]  # type: ignore[index]
        formatters[fid] = config
    return formatters


def _check_unsupported_handler_options(section: dict[str, str]) -> None:
    """Check for handler options that are in allowed set but not supported."""
    if "formatter" in section:
        raise ValueError("handler formatters are not supported")
    if "level" in section:
        raise ValueError("handler level is not supported; use logger levels instead")


def _validate_handler_options(hid: str, section: dict[str, str]) -> None:
    allowed = {"class", "args", "kwargs", "formatter", "level"}
    unknown = set(section) - allowed
    if unknown:
        raise ValueError(
            f"handler {hid!r} has unsupported options: {sorted(unknown)!r}"
        )
    if "class" not in section:
        raise ValueError(f"handler {hid!r} missing class")
    _check_unsupported_handler_options(section)


def _build_handler_config(
    section: dict[str, str],
    defaults: Mapping[str, str],
) -> dict[str, Any]:
    cfg: dict[str, Any] = {"class": section["class"]}
    args_raw = section.get("args")
    cfg["args"] = _expand_placeholders(args_raw or "()", defaults)
    kwargs_raw = section.get("kwargs")
    if kwargs_raw is not None:
        cfg["kwargs"] = _expand_placeholders(kwargs_raw, defaults)
    formatter = section.get("formatter")
    if formatter:
        cfg["formatter"] = formatter
    return cfg


def _parse_handlers(
    sections: dict[str, dict[str, str]],
    defaults: Mapping[str, str],
) -> dict[str, dict[str, Any]]:
    handler_section = sections.get("handlers")
    handler_ids = _split_csv(handler_section.get("keys")) if handler_section else []
    handlers: dict[str, dict[str, Any]] = {}
    for hid in handler_ids:
        section = _require_section(sections, f"handler_{hid}")
        _validate_handler_options(hid, section)
        handlers[hid] = _build_handler_config(section, defaults)
    return handlers


def _validate_logger_options(lid: str, section: dict[str, str]) -> None:
    allowed = {"level", "handlers", "qualname", "propagate"}
    unknown = set(section) - allowed
    if unknown:
        raise ValueError(f"logger {lid!r} has unsupported options: {sorted(unknown)!r}")


def _build_logger_config(section: dict[str, str], qualname: str) -> dict[str, Any]:
    config: dict[str, Any] = {}
    if section.get("level") is not None:
        config["level"] = section["level"]
    if section.get("handlers") is not None:
        config["handlers"] = _split_csv(section.get("handlers"))
    if section.get("propagate") is not None and qualname != "root":
        config["propagate"] = _parse_bool(section["propagate"])
    return config


def _parse_loggers(
    sections: dict[str, dict[str, str]],
) -> tuple[dict[str, dict[str, Any]], dict[str, Any]]:
    logger_section = sections.get("loggers")
    logger_ids = _split_csv(logger_section.get("keys")) if logger_section else []
    loggers: dict[str, dict[str, Any]] = {}
    root_cfg: dict[str, Any] | None = None
    for lid in logger_ids:
        section = _require_section(sections, f"logger_{lid}")
        _validate_logger_options(lid, section)
        qualname = section.get("qualname") or lid
        config = _build_logger_config(section, qualname)
        if qualname == "root":
            root_cfg = config
        else:
            loggers[qualname] = config
    if root_cfg is None:
        raise ValueError("root logger configuration is required")
    root_cfg.setdefault("handlers", [])
    return loggers, root_cfg


def _split_csv(raw: str | None) -> list[str]:
    if not raw:
        return []
    return [value.strip() for value in raw.split(",") if value.strip()]


def _require_section(
    sections: dict[str, dict[str, str]],
    name: str,
) -> dict[str, str]:
    if name not in sections:
        raise ValueError(f"section [{name}] is missing")
    return sections[name]


def _expand_placeholders(value: str, defaults: Mapping[str, str]) -> str:
    if not defaults or "%(" not in value:
        return value

    def replacer(match: re.Match[str]) -> str:
        key = match.group(1)
        if key not in defaults:
            raise ValueError(f"unknown placeholder {key!r} in {value!r}")
        return defaults[key]

    return _PERCENT_PLACEHOLDER.sub(replacer, value)


def _parse_bool(raw: str | None) -> bool:
    if raw is None:
        return False
    value = raw.strip().lower()
    if value in {"1", "true", "yes", "on"}:
        return True
    if value in {"0", "false", "no", "off"}:
        return False
    raise ValueError(f"invalid boolean value {raw!r}")


__all__ = ["fileConfig"]
