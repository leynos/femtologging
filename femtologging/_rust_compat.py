"""Rust extension compatibility layer for optional features."""

from __future__ import annotations

import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

from . import _femtologging_rs as rust


def _make_rotating_fresh_failure_hooks(
    force: object,
    clear: object,
) -> tuple[
    cabc.Callable[[int, str | None], None],
    cabc.Callable[[], None],
]:
    if callable(force) and callable(clear):
        return (
            typ.cast("cabc.Callable[[int, str | None], None]", force),
            typ.cast("cabc.Callable[[], None]", clear),
        )

    def _force(count: int, reason: str | None = None) -> None:
        msg = (
            "rotating fresh-failure hook requires the extension built with the "
            "'python' feature"
        )
        raise RuntimeError(msg)

    def _clear() -> None:
        return

    return _force, _clear


def _make_timed_rotation_hooks(
    setter: object,
    clearer: object,
) -> tuple[
    cabc.Callable[[list[int]], None],
    cabc.Callable[[], None],
]:
    if callable(setter) and callable(clearer):
        return (
            typ.cast("cabc.Callable[[list[int]], None]", setter),
            typ.cast("cabc.Callable[[], None]", clearer),
        )

    def _set(epoch_millis: list[int]) -> None:
        msg = (
            "timed rotation test clock requires the extension built with the "
            "'python' feature"
        )
        raise RuntimeError(msg)

    def _clear() -> None:
        return

    return _set, _clear


def _make_setup_rust_logging(fn: object) -> cabc.Callable[[], None]:
    if callable(fn):
        return typ.cast("cabc.Callable[[], None]", fn)

    def _fallback() -> None:
        msg = (
            "setup_rust_logging requires the extension built with the "
            "'log-compat' Cargo feature"
        )
        raise RuntimeError(msg)

    return _fallback


def _make_runtime_attachment_state(
    fn: object,
) -> cabc.Callable[[str], tuple[list[str], list[str]] | None]:
    if callable(fn):
        return typ.cast("cabc.Callable[[str], tuple[list[str], list[str]] | None]", fn)

    def _fallback(name: str) -> tuple[list[str], list[str]] | None:
        del name
        msg = (
            "runtime attachment state requires the extension built with the "
            "'python' feature"
        )
        raise RuntimeError(msg)

    return _fallback


_force_rotating_fresh_failure_for_test, _clear_rotating_fresh_failure_for_test = (
    _make_rotating_fresh_failure_hooks(
        getattr(rust, "force_rotating_fresh_failure_for_test", None),
        getattr(rust, "clear_rotating_fresh_failure_for_test", None),
    )
)

_set_timed_rotation_test_times_for_test, _clear_timed_rotation_test_times_for_test = (
    _make_timed_rotation_hooks(
        getattr(rust, "set_timed_rotation_test_times_for_test", None),
        getattr(rust, "clear_timed_rotation_test_times_for_test", None),
    )
)

setup_rust_logging = _make_setup_rust_logging(getattr(rust, "setup_rust_logging", None))

_runtime_attachment_state_for_test = _make_runtime_attachment_state(
    getattr(rust, "runtime_attachment_state_for_test", None)
)
