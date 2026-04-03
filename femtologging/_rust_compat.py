"""Rust extension compatibility layer for optional features."""

from __future__ import annotations

import typing as typ

from typing_extensions import TypedDict

if typ.TYPE_CHECKING:
    import collections.abc as cabc

from . import _femtologging_rs as rust


class _RustCompatPayload(TypedDict):
    """Typed schema for Rust extension compatibility layer."""

    _force_rotating_fresh_failure_for_test: cabc.Callable[[int, str | None], None]
    _clear_rotating_fresh_failure_for_test: cabc.Callable[[], None]
    _set_timed_rotation_test_times_for_test: cabc.Callable[[list[int]], None]
    _clear_timed_rotation_test_times_for_test: cabc.Callable[[], None]
    setup_rust_logging: cabc.Callable[[], None]
    setup_rust_tracing: cabc.Callable[[], None]
    _runtime_attachment_state_for_test: cabc.Callable[
        [str], tuple[list[str], list[str]] | None
    ]


def _make_zero_arg_hook(fn: object, error_msg: str = "") -> cabc.Callable[[], None]:
    """Return *fn* cast as a no-arg callable, or a fallback.

    When *error_msg* is non-empty the fallback raises :class:`RuntimeError`;
    otherwise it is a silent no-op.
    """
    if callable(fn):
        return typ.cast("cabc.Callable[[], None]", fn)

    if error_msg:

        def _fallback() -> None:
            raise RuntimeError(error_msg)

        return _fallback

    return lambda: None


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
            "'test-util' feature"
        )
        raise RuntimeError(msg)

    return _force, _make_zero_arg_hook(None)


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
            "'test-util' feature"
        )
        raise RuntimeError(msg)

    return _set, _make_zero_arg_hook(None)


def _make_runtime_attachment_state(
    fn: object,
) -> cabc.Callable[[str], tuple[list[str], list[str]] | None]:
    if callable(fn):
        return typ.cast("cabc.Callable[[str], tuple[list[str], list[str]] | None]", fn)

    def _fallback(name: str) -> tuple[list[str], list[str]] | None:
        del name
        msg = (
            "runtime attachment state requires the extension built with the "
            "'test-util' feature"
        )
        raise RuntimeError(msg)

    return _fallback


def _has_timed_rotation_test_util_support(setter: object, clearer: object) -> bool:
    """Report whether timed rotation test hooks are fully available."""
    return callable(setter) and callable(clearer)


def _initialize_rust_compat() -> _RustCompatPayload:
    """Initialize Rust extension compatibility layer.

    Extracts all optional Rust extension functions and wraps them with
    appropriate fallback behavior. Returns a typed payload of initialized
    module-level variables.
    """
    force_rotating, clear_rotating = _make_rotating_fresh_failure_hooks(
        getattr(rust, "force_rotating_fresh_failure_for_test", None),
        getattr(rust, "clear_rotating_fresh_failure_for_test", None),
    )

    set_timed, clear_timed = _make_timed_rotation_hooks(
        getattr(rust, "set_timed_rotation_test_times_for_test", None),
        getattr(rust, "clear_timed_rotation_test_times_for_test", None),
    )

    return {
        "_force_rotating_fresh_failure_for_test": force_rotating,
        "_clear_rotating_fresh_failure_for_test": clear_rotating,
        "_set_timed_rotation_test_times_for_test": set_timed,
        "_clear_timed_rotation_test_times_for_test": clear_timed,
        "setup_rust_logging": _make_zero_arg_hook(
            getattr(rust, "setup_rust_logging", None),
            "setup_rust_logging requires the extension built with the "
            "'log-compat' Cargo feature",
        ),
        "setup_rust_tracing": _make_zero_arg_hook(
            getattr(rust, "setup_rust_tracing", None),
            "setup_rust_tracing requires the extension built with the "
            "'tracing-compat' Cargo feature",
        ),
        "_runtime_attachment_state_for_test": _make_runtime_attachment_state(
            getattr(rust, "runtime_attachment_state_for_test", None)
        ),
    }


_compat: _RustCompatPayload = _initialize_rust_compat()
_force_rotating_fresh_failure_for_test: cabc.Callable[[int, str | None], None] = (
    _compat["_force_rotating_fresh_failure_for_test"]
)
_clear_rotating_fresh_failure_for_test: cabc.Callable[[], None] = _compat[
    "_clear_rotating_fresh_failure_for_test"
]
_set_timed_rotation_test_times_for_test: cabc.Callable[[list[int]], None] = _compat[
    "_set_timed_rotation_test_times_for_test"
]
_clear_timed_rotation_test_times_for_test: cabc.Callable[[], None] = _compat[
    "_clear_timed_rotation_test_times_for_test"
]
setup_rust_logging: cabc.Callable[[], None] = _compat["setup_rust_logging"]
setup_rust_tracing: cabc.Callable[[], None] = _compat["setup_rust_tracing"]
_runtime_attachment_state_for_test: cabc.Callable[
    [str], tuple[list[str], list[str]] | None
] = _compat["_runtime_attachment_state_for_test"]

# Feature detection: True when the Rust extension was compiled with test-util.
_has_test_util: bool = _has_timed_rotation_test_util_support(
    getattr(rust, "set_timed_rotation_test_times_for_test", None),
    getattr(rust, "clear_timed_rotation_test_times_for_test", None),
)
_has_tracing_compat: bool = hasattr(rust, "setup_rust_tracing")
