"""TLS and backoff configuration for socket handlers.

This module handles parsing and validation of TLS and backoff configuration
from ``dictConfig``-style configuration dictionaries.

These functions are called from :mod:`femtologging.config_socket` when
constructing socket handlers.
"""

from __future__ import annotations

import collections.abc as cabc
import dataclasses
import typing as typ

from .config import _validate_mapping_type, _validate_string_keys


@dataclasses.dataclass(slots=True)
class _TlsConfigParser:
    """Parser for TLS configuration from socket handler kwargs."""

    hid: str

    def parse(self, kwargs: dict[str, object]) -> tuple[str | None, bool] | None:
        """Extract and validate TLS configuration from kwargs."""
        tls_value, domain_kw, insecure_kw = self._pop_tls_kwargs(kwargs)

        if self._is_tls_config_absent(tls_value, domain_kw, insecure_kw):
            return None

        domain, insecure, enabled = self._parse_tls_value(tls_value)
        domain, enabled = self._merge_domain_kwarg(domain, domain_kw, enabled=enabled)
        insecure = self._merge_insecure_kwarg(
            insecure_from_mapping=insecure, insecure_kw=insecure_kw
        )
        enabled = self._enable_for_insecure_kwarg(
            enabled=enabled,
            insecure_kw=insecure_kw,
        )

        if not enabled:
            return None

        self._validate_not_disabled(tls_value)
        return domain, insecure

    @staticmethod
    def _pop_tls_kwargs(
        kwargs: dict[str, object],
    ) -> tuple[object | None, object | None, object | None]:
        """Pop TLS-related kwargs from the configuration mapping."""
        tls_value = kwargs.pop("tls", None)
        domain_kw = kwargs.pop("tls_domain", None)
        insecure_kw = kwargs.pop("tls_insecure", None)
        return tls_value, domain_kw, insecure_kw

    @staticmethod
    def _is_tls_config_absent(
        tls_value: object | None,
        domain_kw: object | None,
        insecure_kw: object | None,
    ) -> bool:
        """Return True if no TLS configuration was provided."""
        return tls_value is None and domain_kw is None and insecure_kw is None

    @staticmethod
    def _enable_for_insecure_kwarg(
        *, enabled: bool, insecure_kw: object | None
    ) -> bool:
        """Enable TLS when the insecure kwarg is supplied."""
        if insecure_kw is None:
            return enabled
        return True

    def _parse_tls_value(
        self, tls_value: object
    ) -> tuple[str | None, bool | None, bool]:
        """Parse the tls kwarg value and return (domain, insecure, enabled)."""
        if isinstance(tls_value, cabc.Mapping):
            domain, insecure = self._parse_mapping(
                typ.cast("cabc.Mapping[object, object]", tls_value)
            )
            return domain, insecure, True
        if isinstance(tls_value, bool):
            return None, None, tls_value
        if tls_value is None:
            return None, None, False
        msg = f"handler {self.hid!r} socket kwargs tls must be a bool or mapping"
        raise TypeError(msg)

    def _parse_mapping(
        self, tls_value: cabc.Mapping[object, object]
    ) -> tuple[str | None, bool | None]:
        """Parse a TLS mapping and return (domain, insecure)."""
        mapping = _validate_mapping_type(
            tls_value, f"handler {self.hid!r} socket kwargs tls"
        )
        mapping = _validate_string_keys(
            mapping, f"handler {self.hid!r} socket kwargs tls"
        )
        unknown = set(mapping) - {"domain", "insecure"}
        if unknown:
            msg = (
                f"handler {self.hid!r} socket kwargs tls has unsupported keys: "
                f"{sorted(unknown)!r}"
            )
            raise ValueError(msg)
        domain = self._extract_domain(mapping)
        insecure = self._extract_insecure(mapping)
        return domain, insecure

    def _extract_domain(self, mapping: cabc.Mapping[str, object]) -> str | None:
        """Extract domain field from TLS mapping."""
        if "domain" not in mapping:
            return None
        domain = mapping["domain"]
        if domain is not None and not isinstance(domain, str):
            msg = f"handler {self.hid!r} socket kwargs tls domain must be a str or None"
            raise TypeError(msg)
        return domain

    def _extract_insecure(self, mapping: cabc.Mapping[str, object]) -> bool | None:
        """Extract insecure field from TLS mapping."""
        if "insecure" not in mapping:
            return None
        insecure_value = mapping["insecure"]
        if not isinstance(insecure_value, bool):
            msg = f"handler {self.hid!r} socket kwargs tls insecure must be a bool"
            raise TypeError(msg)
        return insecure_value

    def _merge_domain_kwarg(
        self, domain: str | None, domain_kw: object | None, *, enabled: bool
    ) -> tuple[str | None, bool]:
        """Merge the tls_domain kwarg with existing domain."""
        if domain_kw is None:
            return domain, enabled
        if not isinstance(domain_kw, str):
            msg = f"handler {self.hid!r} socket kwargs tls_domain must be a str or None"
            raise TypeError(msg)
        domain_kw_str = domain_kw
        if domain is not None and domain_kw_str != domain:
            msg = (
                f"handler {self.hid!r} socket kwargs tls has conflicting domain values"
            )
            raise ValueError(msg)
        return domain_kw_str, True

    def _merge_insecure_kwarg(
        self, *, insecure_from_mapping: bool | None, insecure_kw: object | None
    ) -> bool:
        """Merge the tls_insecure kwarg with existing insecure value."""
        if insecure_kw is None:
            return insecure_from_mapping if insecure_from_mapping is not None else False
        if not isinstance(insecure_kw, bool):
            msg = f"handler {self.hid!r} socket kwargs tls_insecure must be a bool"
            raise TypeError(msg)
        insecure_kw_bool = insecure_kw
        if (
            insecure_from_mapping is not None
            and insecure_kw_bool != insecure_from_mapping
        ):
            msg = (
                f"handler {self.hid!r} socket kwargs tls has conflicting insecure "
                "values"
            )
            raise ValueError(msg)
        return insecure_kw_bool

    def _validate_not_disabled(self, tls_value: object) -> bool:
        """Raise if TLS is disabled but TLS options were supplied."""
        if isinstance(tls_value, bool) and not tls_value:
            msg = (
                f"handler {self.hid!r} socket kwargs tls is disabled but TLS options "
                "were supplied"
            )
            raise ValueError(msg)
        return True


def _pop_socket_tls_kwargs(
    hid: str, kwargs: dict[str, object]
) -> tuple[str | None, bool] | None:
    """Extract and validate TLS configuration from socket handler kwargs."""
    parser = _TlsConfigParser(hid)
    return parser.parse(kwargs)


@dataclasses.dataclass(slots=True)
class _BackoffConfigParser:
    """Parser for backoff configuration from socket handler kwargs."""

    hid: str

    _FIELDS: typ.ClassVar[set[str]] = {
        "base_ms",
        "cap_ms",
        "reset_after_ms",
        "deadline_ms",
    }
    _ALIAS_MAP: typ.ClassVar[dict[str, str]] = {
        "backoff_base_ms": "base_ms",
        "backoff_cap_ms": "cap_ms",
        "backoff_reset_after_ms": "reset_after_ms",
        "backoff_deadline_ms": "deadline_ms",
    }

    def parse(self, kwargs: dict[str, object]) -> dict[str, int | None] | None:
        """Extract and validate backoff configuration from kwargs."""
        backoff_value = kwargs.pop("backoff", None)
        result: dict[str, int | None] = {}

        if backoff_value is not None:
            result = self._parse_mapping(backoff_value)

        result = self._merge_aliases(kwargs, result)

        return result or None

    def _parse_mapping(self, backoff_value: object) -> dict[str, int | None]:
        """Extract backoff values from a mapping."""
        mapping = _validate_mapping_type(
            backoff_value, f"handler {self.hid!r} socket kwargs backoff"
        )
        mapping = _validate_string_keys(
            mapping, f"handler {self.hid!r} socket kwargs backoff"
        )
        unknown = set(mapping) - self._FIELDS
        if unknown:
            msg = (
                f"handler {self.hid!r} socket kwargs backoff has unsupported keys: "
                f"{sorted(unknown)!r}"
            )
            raise ValueError(msg)

        return {
            key: self._coerce_value(key, mapping[key])
            for key in self._FIELDS
            if key in mapping
        }

    def _merge_aliases(
        self, kwargs: dict[str, object], result: dict[str, int | None]
    ) -> dict[str, int | None]:
        """Merge backoff alias kwargs with mapping values."""
        merged = dict(result)
        for alias, target in self._ALIAS_MAP.items():
            if alias not in kwargs:
                continue
            value = self._coerce_value(alias, kwargs.pop(alias))
            self._check_conflict(target, merged.get(target), value)
            merged[target] = value
        return merged

    def _coerce_value(self, key: str, value: object) -> int | None:
        """Coerce a backoff value to int or None, validating type and range."""
        if value is None:
            return None
        if isinstance(value, bool) or not isinstance(value, int):
            msg = f"handler {self.hid!r} socket kwargs {key} must be an int or None"
            raise TypeError(msg)
        if value < 0:
            msg = f"handler {self.hid!r} socket kwargs {key} must be non-negative"
            raise ValueError(msg)
        return value

    def _check_conflict(
        self, target: str, existing: int | None, new: int | None
    ) -> None:
        """Raise ValueError if conflicting backoff values are detected."""
        either_is_none = existing is None or new is None
        if either_is_none or existing == new:
            return
        msg = f"handler {self.hid!r} socket kwargs backoff {target} conflict"
        raise ValueError(msg)


def _pop_socket_backoff_kwargs(
    hid: str, kwargs: dict[str, object]
) -> dict[str, int | None] | None:
    """Extract and validate backoff configuration from socket handler kwargs."""
    parser = _BackoffConfigParser(hid)
    return parser.parse(kwargs)
