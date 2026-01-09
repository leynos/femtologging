"""Rust extension compatibility layer for optional features."""

from __future__ import annotations

import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

from . import _femtologging_rs as rust

_force_rotating_fresh_failure = getattr(
    rust, "force_rotating_fresh_failure_for_test", None
)
_clear_rotating_fresh_failure = getattr(
    rust, "clear_rotating_fresh_failure_for_test", None
)
_setup_rust_logging = getattr(rust, "setup_rust_logging", None)

if callable(_force_rotating_fresh_failure) and callable(_clear_rotating_fresh_failure):
    _force_rotating_fresh_failure_for_test = typ.cast(
        "cabc.Callable[[int, str | None], None]",
        _force_rotating_fresh_failure,
    )
    _clear_rotating_fresh_failure_for_test = typ.cast(
        "cabc.Callable[[], None]",
        _clear_rotating_fresh_failure,
    )
else:
    # Feature disabled: expose no-ops that fail loudly when invoked.

    def _force_rotating_fresh_failure_for_test(
        count: int, reason: str | None = None
    ) -> None:
        msg = (
            "rotating fresh-failure hook requires the extension built with the "
            "'python' feature"
        )
        raise RuntimeError(msg)

    def _clear_rotating_fresh_failure_for_test() -> None:
        return


if callable(_setup_rust_logging):
    setup_rust_logging = typ.cast(
        "cabc.Callable[[], None]",
        _setup_rust_logging,
    )
else:

    def setup_rust_logging() -> None:
        msg = (
            "setup_rust_logging requires the extension built with the "
            "'log-compat' Cargo feature"
        )
        raise RuntimeError(msg)
