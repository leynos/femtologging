"""Check that an installed wheel is production-built and imports in isolation."""

from __future__ import annotations

import importlib
import importlib.machinery
import sys
from pathlib import Path


def _native_location_errors(native_location: object) -> list[str]:
    """Return violations when a native module path is outside this environment.

    Parameters
    ----------
    native_location : object
        The value reported by the native module's ``__file__`` attribute.

    Returns
    -------
    list[str]
        Human-readable path or extension-suffix violations.

    Examples
    --------
    >>> _native_location_errors(None)
    ['native extension has no filesystem origin']
    """
    if not isinstance(native_location, str):
        return ["native extension has no filesystem origin"]

    native_path = Path(native_location).resolve()
    environment = Path(sys.prefix).resolve()
    errors: list[str] = []
    try:
        native_path.relative_to(environment)
    except ValueError:
        errors.append(f"native extension is outside sys.prefix: {native_path}")
    has_extension_suffix = any(
        native_path.name.endswith(suffix)
        for suffix in importlib.machinery.EXTENSION_SUFFIXES
    )
    if not has_extension_suffix:
        errors.append(f"native module does not use an extension suffix: {native_path}")
    return errors


def release_wheel_errors() -> list[str]:
    """Return checks that fail when this process is not using a release wheel.

    Returns
    -------
    list[str]
        Human-readable violations. An empty list means that the imported
        extension belongs to this environment and contains no test hooks.
    """
    import femtologging

    native = importlib.import_module("femtologging._femtologging_rs")
    errors = _native_location_errors(getattr(native, "__file__", None))

    hooks = (
        "set_timed_rotation_test_times_for_test",
        "clear_timed_rotation_test_times_for_test",
    )
    exposed = [name for name in hooks if hasattr(native, name)]
    if exposed:
        errors.append(f"native extension exposes test hooks: {', '.join(exposed)}")
    if femtologging._has_test_util is not False:
        errors.append("femtologging._has_test_util must be False")
    return errors


def main() -> int:
    """Print release-wheel violations and return a process status.

    Returns
    -------
    int
        ``0`` when no violations are found; ``1`` when violations are found.

    Examples
    --------
    Run the probe after installing a release wheel in the active environment:

    >>> main()  # doctest: +SKIP
    release wheel probe passed
    0
    """
    errors = release_wheel_errors()
    if errors:
        for error in errors:
            print(f"release wheel probe: {error}", file=sys.stderr)
        return 1
    print("release wheel probe passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
