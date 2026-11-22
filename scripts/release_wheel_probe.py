"""Check that an installed wheel is production-built and delivers records.

The location checks prove that the interpreter has loaded this environment's
native extension rather than the development build. They say nothing about
whether that extension works, so the probe also sends one record through the
native HTTP worker to a loopback endpoint. A wheel whose dependencies or TLS
or transport configuration are wrong passes the location checks and fails the
delivery check.

The checks themselves live in ``release_wheel_checks`` so that each module
stays below the repository file-length limit. The workflows run this script
with ``python -I`` from an empty directory, which also drops the script's own
directory from ``sys.path``; that one directory is restored below. It holds
only the probe's own checks, never the wheel under test.
"""

from __future__ import annotations

import importlib
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from release_wheel_checks import (
    _native_location_errors,
    _native_surface_errors,
    _package_surface_errors,
    _stub_surface_errors,
    http_behaviour_errors,
)


def _resolve_probe_inputs(
    native: object | None, package: object | None
) -> tuple[object | None, object | None]:
    """Fill in the native module and package that the caller left unspecified.

    Parameters
    ----------
    native : object | None
        Injected native module, or ``None`` to import the installed one.
    package : object | None
        Injected ``femtologging`` package, or ``None`` to import it.

    Returns
    -------
    tuple[object | None, object | None]
        The native module and package to inspect.

    Examples
    --------
    >>> native = object()
    >>> _resolve_probe_inputs(native, object())[0] is native
    True
    """
    if package is None:
        import femtologging

        package = femtologging
    if native is None:
        native = importlib.import_module("femtologging._femtologging_rs")
    return native, package


def release_wheel_errors(
    *, native: object | None = None, package: object | None = None
) -> list[str]:
    """Return checks that fail when this process is not using a release wheel.

    Parameters
    ----------
    native : object | None
        Native extension module to inspect. Defaults to the module installed
        in this process; tests inject a stand-in so no shared import state is
        replaced.
    package : object | None
        ``femtologging`` package to inspect. Defaults to the imported package.

    Returns
    -------
    list[str]
        Human-readable violations. An empty list means that the imported
        extension belongs to this environment, contains no test hooks, delivered
        a record over HTTP, and that its stub declares only the production API.
    """
    native, package = _resolve_probe_inputs(native, package)
    errors = _native_location_errors(getattr(native, "__file__", None))
    errors.extend(_native_surface_errors(native))
    errors.extend(_package_surface_errors(package))
    errors.extend(_stub_surface_errors(package))
    if getattr(package, "_has_test_util", None) is not False:
        errors.append("femtologging._has_test_util must be False")
    if not errors:
        errors.extend(http_behaviour_errors())
    return errors


def main(*, native: object | None = None, package: object | None = None) -> int:
    """Print release-wheel violations and return a process status.

    Parameters
    ----------
    native : object | None
        Native extension module to inspect. Defaults to the installed module.
    package : object | None
        ``femtologging`` package to inspect. Defaults to the imported package.

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
    errors = release_wheel_errors(native=native, package=package)
    if errors:
        for error in errors:
            print(f"release wheel probe: {error}", file=sys.stderr)
        return 1
    print("release wheel probe passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
