"""Check that an installed wheel is production-built and imports in isolation."""

from __future__ import annotations

import importlib
import importlib.machinery
import re
import sys
from pathlib import Path

# Test-only names that follow none of the shapes below.
_TEST_ONLY_EXACT_NAMES = frozenset({"_stream_waiter_count", "AttachmentIdentity"})
_TEST_ONLY_NAME_PREFIXES = ("_emit_rust_", "_install_test_")
_TEST_ONLY_NAME_SUFFIX = "_for_test"
_STUB_DECLARATION_RE = re.compile(
    r"^\s*(?:def\s+)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*[:=(]",
    re.MULTILINE,
)


def _is_test_only_name(name: str) -> bool:
    """Return whether *name* belongs to a test-only hook or helper.

    Parameters
    ----------
    name : str
        Attribute or stub declaration name to classify.

    Returns
    -------
    bool
        ``True`` for names that only a ``test-util`` build may expose.

    Examples
    --------
    >>> _is_test_only_name("wait_for_flush_entry_for_test")
    True
    >>> _is_test_only_name("setup_rust_logging")
    False
    """
    return (
        name in _TEST_ONLY_EXACT_NAMES
        or name.endswith(_TEST_ONLY_NAME_SUFFIX)
        or name.startswith(_TEST_ONLY_NAME_PREFIXES)
    )


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


def _native_surface_errors(native: object) -> list[str]:
    """Return violations for test-only attributes on the native surface.

    Both the module namespace and the ``FemtoLogger`` type are checked, because
    an attachment hook is a method rather than a module function.

    Parameters
    ----------
    native : object
        The imported native extension module.

    Returns
    -------
    list[str]
        Human-readable violations for module functions and logger methods.

    Examples
    --------
    >>> _native_surface_errors(object())
    []
    """
    errors: list[str] = []
    module_hooks = sorted(name for name in dir(native) if _is_test_only_name(name))
    if module_hooks:
        errors.append("native extension exposes test hooks: " + ", ".join(module_hooks))
    logger_type = getattr(native, "FemtoLogger", None)
    if logger_type is not None:
        method_hooks = sorted(
            name for name in dir(logger_type) if _is_test_only_name(name)
        )
        if method_hooks:
            errors.append(
                "FemtoLogger exposes test-only methods: " + ", ".join(method_hooks)
            )
    return errors


def _stub_surface_errors(package: object) -> list[str]:
    """Return violations for test-only declarations in the shipped stub.

    A stub is the published contract of the wheel. Declaring a hook that only a
    ``test-util`` build provides would promise production users an API they
    cannot call, so the stub must omit the same names as the native module.

    Parameters
    ----------
    package : object
        The imported ``femtologging`` package.

    Returns
    -------
    list[str]
        Human-readable violations found in ``_femtologging_rs.pyi``.

    Examples
    --------
    >>> _stub_surface_errors(object())
    ['femtologging package has no filesystem origin']
    """
    package_file = getattr(package, "__file__", None)
    if not isinstance(package_file, str):
        return ["femtologging package has no filesystem origin"]
    stub = Path(package_file).with_name("_femtologging_rs.pyi")
    if not stub.is_file():
        return [f"release wheel omits the type stub: {stub}"]
    declarations = {
        match.group("name")
        for match in _STUB_DECLARATION_RE.finditer(stub.read_text(encoding="utf-8"))
    }
    hooks = sorted(name for name in declarations if _is_test_only_name(name))
    if hooks:
        return ["type stub declares test-only hooks: " + ", ".join(hooks)]
    return []


def release_wheel_errors() -> list[str]:
    """Return checks that fail when this process is not using a release wheel.

    Returns
    -------
    list[str]
        Human-readable violations. An empty list means that the imported
        extension belongs to this environment and contains no test hooks, and
        that its stub declares only the production API.
    """
    import femtologging

    native = importlib.import_module("femtologging._femtologging_rs")
    errors = _native_location_errors(getattr(native, "__file__", None))
    errors.extend(_native_surface_errors(native))
    errors.extend(_stub_surface_errors(femtologging))
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
