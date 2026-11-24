"""Rust extension compatibility layer for optional features."""

from __future__ import annotations

import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

from . import _femtologging_rs as rust


class _RustCompatPayload(typ.TypedDict, total=False):
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
    """Return *fn* as a callable, or a fallback that raises/no-ops per *error_msg*."""
    if callable(fn):
        return typ.cast("cabc.Callable[[], None]", fn)

    if error_msg:

        def _fallback() -> None:
            """Raise the captured missing-feature error."""
            raise RuntimeError(error_msg)

        return _fallback

    return lambda: None


def _make_rotating_fresh_failure_hooks(
    force: object,
    clear: object,
) -> (
    tuple[
        cabc.Callable[[int, str | None], None],
        cabc.Callable[[], None],
    ]
    | None
):
    """Pair *force*/*clear* when both exist, else report no test hook."""
    if callable(force) and callable(clear):
        return (
            typ.cast("cabc.Callable[[int, str | None], None]", force),
            typ.cast("cabc.Callable[[], None]", clear),
        )
    return None


def _make_timed_rotation_hooks(
    setter: object,
    clearer: object,
) -> (
    tuple[
        cabc.Callable[[list[int]], None],
        cabc.Callable[[], None],
    ]
    | None
):
    """Pair *setter*/*clearer* when both exist, else report no test hook."""
    if callable(setter) and callable(clearer):
        return (
            typ.cast("cabc.Callable[[list[int]], None]", setter),
            typ.cast("cabc.Callable[[], None]", clearer),
        )
    return None


def _make_runtime_attachment_state(
    fn: object,
) -> cabc.Callable[[str], tuple[list[str], list[str]] | None] | None:
    """Return *fn* when it exists, else report no test hook."""
    if callable(fn):
        return typ.cast("cabc.Callable[[str], tuple[list[str], list[str]] | None]", fn)
    return None


def _has_timed_rotation_test_util_support(setter: object, clearer: object) -> bool:
    """Report whether timed rotation test hooks are fully available."""
    return callable(setter) and callable(clearer)


def _initialize_rust_compat() -> _RustCompatPayload:
    """Extract the optional Rust extension functions that this build provides."""
    payload: _RustCompatPayload = {
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
    }
    rotating = _make_rotating_fresh_failure_hooks(
        getattr(rust, "force_rotating_fresh_failure_for_test", None),
        getattr(rust, "clear_rotating_fresh_failure_for_test", None),
    )
    if rotating is not None:
        payload["_force_rotating_fresh_failure_for_test"] = rotating[0]
        payload["_clear_rotating_fresh_failure_for_test"] = rotating[1]
    timed = _make_timed_rotation_hooks(
        getattr(rust, "set_timed_rotation_test_times_for_test", None),
        getattr(rust, "clear_timed_rotation_test_times_for_test", None),
    )
    if timed is not None:
        payload["_set_timed_rotation_test_times_for_test"] = timed[0]
        payload["_clear_timed_rotation_test_times_for_test"] = timed[1]
    runtime_state = _make_runtime_attachment_state(
        getattr(rust, "runtime_attachment_state_for_test", None)
    )
    if runtime_state is not None:
        payload["_runtime_attachment_state_for_test"] = runtime_state
    return payload


_compat: _RustCompatPayload = _initialize_rust_compat()
setup_rust_logging: cabc.Callable[[], None] = _compat["setup_rust_logging"]
setup_rust_tracing: cabc.Callable[[], None] = _compat["setup_rust_tracing"]

# The test hooks are defined only when the native extension provides them. A
# production build therefore has no `_for_test` names in this module either,
# rather than raising or no-op compatibility shims.
if "_force_rotating_fresh_failure_for_test" in _compat:
    _force_rotating_fresh_failure_for_test: cabc.Callable[[int, str | None], None] = (
        _compat["_force_rotating_fresh_failure_for_test"]
    )
    _clear_rotating_fresh_failure_for_test: cabc.Callable[[], None] = _compat[
        "_clear_rotating_fresh_failure_for_test"
    ]
if "_set_timed_rotation_test_times_for_test" in _compat:
    _set_timed_rotation_test_times_for_test: cabc.Callable[[list[int]], None] = _compat[
        "_set_timed_rotation_test_times_for_test"
    ]
    _clear_timed_rotation_test_times_for_test: cabc.Callable[[], None] = _compat[
        "_clear_timed_rotation_test_times_for_test"
    ]
if "_runtime_attachment_state_for_test" in _compat:
    _runtime_attachment_state_for_test: cabc.Callable[
        [str], tuple[list[str], list[str]] | None
    ] = _compat["_runtime_attachment_state_for_test"]

# Feature detection: True when the Rust extension was compiled with test-util.
_has_test_util: bool = _has_timed_rotation_test_util_support(
    getattr(rust, "set_timed_rotation_test_times_for_test", None),
    getattr(rust, "clear_timed_rotation_test_times_for_test", None),
)
_has_tracing_compat: bool = hasattr(rust, "setup_rust_tracing")
