"""Pure checks used by the release-wheel probe.

The probe runs with ``python -I`` from an empty directory, so this module must
stay importable without the repository on ``sys.path`` and must not import
``femtologging`` at module scope: importing the package under test is the
probe's job, and it happens only when a check needs it.
"""

from __future__ import annotations

import ast
import importlib.machinery
import sys
import typing as typ
from pathlib import Path

if typ.TYPE_CHECKING:
    import collections.abc as cabc


_TEST_ONLY_EXACT_NAMES = frozenset({
    "_rust_log_enabled",
    "_rust_log_max_level",
    "_stream_waiter_count",
    "AttachmentIdentity",
})
_TEST_ONLY_NAME_PREFIXES = ("_emit_rust_", "_install_test_")
_TEST_ONLY_NAME_SUFFIX = "_for_test"


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


def _reexported_names(aliases: cabc.Iterable[ast.alias]) -> set[str]:
    """Return the names *aliases* re-export under an identical alias.

    Parameters
    ----------
    aliases : collections.abc.Iterable[ast.alias]
        Imported names from a ``from`` statement.

    Returns
    -------
    set[str]
        Names imported as themselves, which is how a stub re-exports them.

    Examples
    --------
    >>> _reexported_names([ast.alias(name="Token", asname="Token")])
    {'Token'}
    >>> _reexported_names([ast.alias(name="Token", asname=None)])
    set()
    """
    return {
        alias.asname
        for alias in aliases
        if alias.asname is not None and alias.asname == alias.name
    }


def _string_values(value: ast.expr) -> set[str]:
    """Return the string literals in a list, tuple or set expression."""
    match value:
        case ast.List(elts=elts) | ast.Set(elts=elts) | ast.Tuple(elts=elts):
            return {
                element.value
                for element in elts
                if isinstance(element, ast.Constant) and isinstance(element.value, str)
            }
    return set()


def _exports_all(targets: list[ast.expr]) -> bool:
    """Return whether an assignment targets the module's ``__all__``."""
    return any(
        isinstance(target, ast.Name) and target.id == "__all__" for target in targets
    )


def _exported_names(tree: ast.AST) -> set[str]:
    """Return the statically listed names in every ``__all__`` assignment.

    ``__all__`` makes an imported name public, so a stub can publish a test
    hook with a plain import plus an ``__all__`` entry. Only literal string
    lists are readable statically; a computed ``__all__`` cannot be checked
    here.

    Parameters
    ----------
    tree : ast.AST
        Parsed type stub.

    Returns
    -------
    set[str]
        Names listed in a static ``__all__`` assignment.

    Examples
    --------
    >>> sorted(_exported_names(ast.parse('__all__ = ["Token", "Other"]')))
    ['Other', 'Token']
    """
    exported: set[str] = set()
    for node in ast.walk(tree):
        match node:
            case ast.Assign(targets=targets, value=value) if _exports_all(targets):
                exported.update(_string_values(value))
            case ast.AnnAssign(target=ast.Name(id="__all__"), value=value) if value:
                exported.update(_string_values(value))
    return exported


def _stub_declaration_names(tree: ast.AST) -> set[str]:
    """Return every name *tree* declares or explicitly re-exports.

    Parsing the stub rather than scanning lines keeps the check correct for
    syntax that is not a bare ``name:`` line, such as a PEP 695 generic class
    whose name is followed by ``[``. Type parameters are not declarations of
    the module's API, so they are not collected. A ``from x import Y as Y``
    alias is a stub's way of re-exporting ``Y``; a plain import stays private
    and is not collected unless the stub lists it in ``__all__``.

    Parameters
    ----------
    tree : ast.AST
        Parsed type stub.

    Returns
    -------
    set[str]
        Names declared anywhere in the stub, including nested ones.

    Examples
    --------
    >>> sorted(_stub_declaration_names(ast.parse("class Token[T]: ...")))
    ['Token']
    >>> _stub_declaration_names(ast.parse("from .mod import Token as Token"))
    {'Token'}
    """
    names: set[str] = _exported_names(tree)
    for node in ast.walk(tree):
        match node:
            case ast.AsyncFunctionDef() | ast.ClassDef() | ast.FunctionDef():
                names.add(node.name)
            case ast.AnnAssign(target=ast.Name(id=name)):
                names.add(name)
            case ast.Assign(targets=targets):
                names.update(
                    target.id for target in targets if isinstance(target, ast.Name)
                )
            case ast.TypeAlias(name=ast.Name(id=name)):
                names.add(name)
            case ast.ImportFrom(names=aliases):
                names.update(_reexported_names(aliases))
    return names


def _package_surface_errors(package: object) -> list[str]:
    """Return violations for test-only helpers exported by the package.

    The native module and the shipped stub are not the only surfaces a wheel
    exposes: ``femtologging`` and its private compatibility module define
    pure-Python helpers, and a production build must omit the fault-injection
    names there rather than keep raising or no-op shims.

    Parameters
    ----------
    package : object
        The imported ``femtologging`` package.

    Returns
    -------
    list[str]
        Human-readable violations for package-level test hooks.

    Examples
    --------
    >>> _package_surface_errors(object())
    []
    """
    surfaces = [package]
    compat = getattr(package, "_rust_compat", None)
    if compat is not None:
        surfaces.append(compat)
    hooks = sorted(
        name
        for surface in surfaces
        for name in dir(surface)
        if _is_test_only_name(name)
    )
    if hooks:
        return ["femtologging exposes test-only hooks: " + ", ".join(hooks)]
    return []


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
    try:
        declarations = _stub_declaration_names(
            ast.parse(stub.read_text(encoding="utf-8"))
        )
    except SyntaxError:
        return [f"release wheel type stub cannot be parsed: {stub}"]
    hooks = sorted(name for name in declarations if _is_test_only_name(name))
    if hooks:
        return ["type stub declares test-only hooks: " + ", ".join(hooks)]
    return []
