"""TLS and backoff configuration for socket handlers.

This module handles parsing and validation of TLS and backoff configuration
from ``dictConfig``-style configuration dictionaries.

These functions are called from :mod:`femtologging.config_socket` when
constructing socket handlers.
"""

from __future__ import annotations

import collections.abc as cabc
import dataclasses
import types
import typing as typ

from .config import _validate_mapping_type, _validate_string_keys


@dataclasses.dataclass(slots=True)
class _TlsMergedState:
    """Merged TLS configuration state after parsing all sources."""

    domain: str | None
    insecure: bool
    enabled: bool


def _validate_type_or_none(
    value: object, expected_type: type, hid: str, field: str
) -> None:
    """Validate that value is of expected_type or None."""
    if value is not None and not isinstance(value, expected_type):
        type_name = expected_type.__name__
        msg = f"handler {hid!r} socket kwargs {field} must be a {type_name} or None"
        raise TypeError(msg)


def _finalize_tls_config(
    hid: str,
    state: _TlsMergedState,
    *,
    insecure_kw: object | None,
    tls_value: object,
) -> tuple[str | None, bool] | None:
    """Finalise TLS configuration after merging all sources."""
    if insecure_kw is not None:
        state.enabled = True

    if not state.enabled:
        return None

    _validate_tls_not_disabled(hid, tls_value)

    return state.domain, state.insecure


def _pop_socket_tls_kwargs(
    hid: str, kwargs: dict[str, object]
) -> tuple[str | None, bool] | None:
    """Extract and validate TLS configuration from socket handler kwargs."""
    tls_value = kwargs.pop("tls", None)
    domain_kw = kwargs.pop("tls_domain", None)
    insecure_kw = kwargs.pop("tls_insecure", None)
    no_tls_config_provided = (
        tls_value is None and domain_kw is None and insecure_kw is None
    )
    if no_tls_config_provided:
        return None

    domain, insecure, enabled = _parse_tls_value(hid, tls_value)
    domain, enabled = _merge_tls_domain_kwarg(
        hid,
        domain,
        domain_kw,
        enabled=enabled,
    )
    insecure = _merge_tls_insecure_kwarg(
        hid,
        insecure_from_mapping=insecure,
        insecure_kw=insecure_kw,
    )
    state = _TlsMergedState(domain=domain, insecure=insecure, enabled=enabled)
    return _finalize_tls_config(
        hid, state, insecure_kw=insecure_kw, tls_value=tls_value
    )


def _parse_tls_value(
    hid: str, tls_value: object
) -> tuple[str | None, bool | None, bool]:
    """Parse the tls kwarg value and return (domain, insecure, enabled)."""
    if isinstance(tls_value, cabc.Mapping):
        domain, insecure = _parse_tls_mapping(
            hid, typ.cast("cabc.Mapping[object, object]", tls_value)
        )
        return domain, insecure, True
    if isinstance(tls_value, bool):
        return None, None, tls_value
    if tls_value is None:
        return None, None, False
    msg = f"handler {hid!r} socket kwargs tls must be a bool or mapping"
    raise TypeError(msg)


def _parse_tls_mapping(
    hid: str, tls_value: cabc.Mapping[object, object]
) -> tuple[str | None, bool | None]:
    """Parse a TLS mapping and return (domain, insecure)."""
    mapping = _validate_mapping_type(tls_value, f"handler {hid!r} socket kwargs tls")
    mapping = _validate_string_keys(mapping, f"handler {hid!r} socket kwargs tls")
    unknown = set(mapping) - {"domain", "insecure"}
    if unknown:
        msg = (
            f"handler {hid!r} socket kwargs tls has unsupported keys: "
            f"{sorted(unknown)!r}"
        )
        raise ValueError(msg)
    domain = _extract_tls_domain_from_mapping(hid, mapping)
    insecure = _extract_tls_insecure_from_mapping(hid, mapping)
    return domain, insecure


def _extract_tls_domain_from_mapping(
    hid: str, tls_mapping: cabc.Mapping[str, object]
) -> str | None:
    """Extract the domain field from a TLS mapping."""
    if "domain" not in tls_mapping:
        return None
    domain = tls_mapping["domain"]
    _validate_type_or_none(domain, str, hid, "tls domain")
    return typ.cast("str | None", domain)


def _extract_tls_insecure_from_mapping(
    hid: str, tls_mapping: cabc.Mapping[str, object]
) -> bool | None:
    """Extract the insecure field from a TLS mapping."""
    if "insecure" not in tls_mapping:
        return None
    insecure_value = tls_mapping["insecure"]
    if not isinstance(insecure_value, bool):
        msg = f"handler {hid!r} socket kwargs tls insecure must be a bool"
        raise TypeError(msg)
    return insecure_value


def _merge_tls_domain_kwarg(
    hid: str,
    domain: str | None,
    domain_kw: object | None,
    *,
    enabled: bool,
) -> tuple[str | None, bool]:
    """Merge the tls_domain kwarg with existing domain from mapping."""
    if domain_kw is None:
        return domain, enabled
    _validate_type_or_none(domain_kw, str, hid, "tls_domain")
    domain_kw_str = typ.cast("str", domain_kw)
    if domain is not None and domain_kw_str != domain:
        msg = f"handler {hid!r} socket kwargs tls has conflicting domain values"
        raise ValueError(msg)
    return domain_kw_str, True


def _merge_tls_insecure_kwarg(
    hid: str,
    *,
    insecure_from_mapping: bool | None,
    insecure_kw: object | None,
) -> bool:
    """Merge the tls_insecure kwarg with existing insecure value from mapping."""
    if insecure_kw is None:
        return insecure_from_mapping if insecure_from_mapping is not None else False
    if not isinstance(insecure_kw, bool):
        msg = f"handler {hid!r} socket kwargs tls_insecure must be a bool"
        raise TypeError(msg)
    insecure_kw_bool = insecure_kw
    if insecure_from_mapping is not None and insecure_kw_bool != insecure_from_mapping:
        msg = f"handler {hid!r} socket kwargs tls has conflicting insecure values"
        raise ValueError(msg)
    return insecure_kw_bool


def _validate_tls_not_disabled(hid: str, tls_value: object) -> None:
    """Raise if TLS is disabled but TLS options were supplied."""
    if isinstance(tls_value, bool) and not tls_value:
        msg = (
            f"handler {hid!r} socket kwargs tls is disabled but TLS options were "
            "supplied"
        )
        raise ValueError(msg)


def _pop_socket_backoff_kwargs(
    hid: str, kwargs: dict[str, object]
) -> dict[str, int | None] | None:
    """Extract and validate backoff configuration from socket handler kwargs."""
    backoff_value = kwargs.pop("backoff", None)
    overrides: dict[str, int | None] = {}

    if backoff_value is not None:
        overrides = _extract_backoff_mapping_values(hid, backoff_value)

    overrides = _merge_backoff_alias_values(hid, kwargs, overrides)

    if not overrides:
        return None

    return overrides


def _extract_backoff_mapping_values(
    hid: str, backoff_value: object
) -> dict[str, int | None]:
    """Extract backoff values from a mapping."""
    mapping = _validate_mapping_type(
        backoff_value, f"handler {hid!r} socket kwargs backoff"
    )
    mapping = _validate_string_keys(mapping, f"handler {hid!r} socket kwargs backoff")
    unknown = set(mapping) - {
        "base_ms",
        "cap_ms",
        "reset_after_ms",
        "deadline_ms",
    }
    if unknown:
        msg = (
            f"handler {hid!r} socket kwargs backoff has unsupported keys:"
            f" {sorted(unknown)!r}"
        )
        raise ValueError(msg)

    return {
        key: _extract_backoff_key(hid, key, mapping)
        for key in ("base_ms", "cap_ms", "reset_after_ms", "deadline_ms")
        if key in mapping
    }


def _extract_backoff_key(
    hid: str, key: str, mapping: cabc.Mapping[str, object]
) -> int | None:
    """Extract a single backoff key from a mapping."""
    return _coerce_backoff_value(hid, key, mapping[key])


_BACKOFF_ALIAS_MAP: typ.Final[cabc.Mapping[str, str]] = types.MappingProxyType({
    "backoff_base_ms": "base_ms",
    "backoff_cap_ms": "cap_ms",
    "backoff_reset_after_ms": "reset_after_ms",
    "backoff_deadline_ms": "deadline_ms",
})


def _merge_backoff_alias_values(
    hid: str,
    kwargs: dict[str, object],
    overrides: dict[str, int | None],
) -> dict[str, int | None]:
    """Merge backoff alias kwargs (backoff_base_ms, etc.) with mapping values."""
    merged = dict(overrides)
    for alias, target in _BACKOFF_ALIAS_MAP.items():
        present = alias in kwargs
        value = _extract_backoff_alias(hid, kwargs, alias)
        # Merge when: (1) alias key was explicitly provided (present=True), even
        # if _extract_backoff_alias returns None (explicit None override), or
        # (2) _extract_backoff_alias resolved a non-None value. Both conditions
        # require _check_backoff_conflict validation before setting merged[target].
        if present or value is not None:
            _check_backoff_conflict(hid, target, merged.get(target), value)
            merged[target] = value
    return merged


def _extract_backoff_alias(
    hid: str,
    kwargs: dict[str, object],
    alias: str,
) -> int | None:
    """Extract and coerce a backoff alias kwarg, returning None if not present."""
    if alias not in kwargs:
        return None
    return _coerce_backoff_value(hid, alias, kwargs.pop(alias))


def _check_backoff_conflict(
    hid: str, target: str, existing: int | None, new: int | None
) -> None:
    """Raise ValueError if conflicting backoff values are detected."""
    no_conflict = existing is None or new is None or existing == new
    if no_conflict:
        return
    msg = f"handler {hid!r} socket kwargs backoff {target} conflict"
    raise ValueError(msg)


def _coerce_backoff_value(hid: str, key: str, value: object) -> int | None:
    """Coerce a backoff value to int or None, validating type and range."""
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        msg = f"handler {hid!r} socket kwargs {key} must be an int or None"
        raise TypeError(msg)
    if value < 0:
        msg = f"handler {hid!r} socket kwargs {key} must be non-negative"
        raise ValueError(msg)
    return value
