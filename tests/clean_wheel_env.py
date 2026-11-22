"""Prove a built wheel imports from a clean, dependency-free environment.

The packaged distribution declares no runtime dependencies, so every import it
performs at start-up must resolve from the standard library, the package
itself, or the native extension. The helpers here install an already-built
wheel into a fresh virtual environment that has no system site packages and
then import the package from outside the checkout. That detects an undeclared
runtime dependency such as ``typing_extensions`` before it reaches a deployer.
"""

from __future__ import annotations

import importlib.util
import os
import subprocess  # ruff: ignore[suspicious-subprocess-import] fixed local interpreter and wheel paths.
import sys
import typing as typ

if typ.TYPE_CHECKING:
    from pathlib import Path

_VENV_CREATION_TIMEOUT_SECONDS = 120
_WHEEL_INSTALL_TIMEOUT_SECONDS = 300
_IMPORT_PROBE_TIMEOUT_SECONDS = 60
_REQUIRED_VENV_MODULES = ("venv", "ensurepip")

# The probe runs in a child interpreter, so it may depend on nothing but the
# standard library. It asserts the clean state *before* importing the package:
# importing first would let a missing dependency masquerade as a package import
# error, which is the very failure this smoke test exists to detect.
_CLEAN_IMPORT_PROBE = """\
import importlib.metadata
import importlib.util
import pathlib
import re
import sys
import sysconfig

assert importlib.util.find_spec("typing_extensions") is None, (
    "typing_extensions must not be importable in a clean environment"
)


def normalize(name):
    return re.sub(r"[-_.]+", "-", name).lower()


installed = {
    normalize(distribution.metadata["Name"] or "")
    for distribution in importlib.metadata.distributions()
}
unexpected = installed - {"femtologging", "pip"}
assert not unexpected, f"unexpected distributions installed: {sorted(unexpected)!r}"

import femtologging
import femtologging._femtologging_rs as rust_extension
import femtologging.config
import femtologging.config_sections
import femtologging.config_socket
import femtologging.file_config

assert "typing_extensions" not in sys.modules, (
    "importing femtologging must not load typing_extensions"
)

install_directories = [
    pathlib.Path(sysconfig.get_paths()[key]).resolve()
    for key in ("purelib", "platlib")
]
for module in (femtologging, rust_extension):
    module_path = pathlib.Path(module.__file__).resolve()
    assert any(
        module_path.is_relative_to(directory) for directory in install_directories
    ), f"{module.__name__} was imported from {module_path}, not an install directory"
"""


def _find_spec(name: str) -> object | None:
    """Return the import specification for *name*, if it is importable."""
    return importlib.util.find_spec(name)


def missing_clean_venv_modules() -> tuple[str, ...]:
    """Return the stdlib modules needed to create a pip-equipped virtualenv."""
    return tuple(name for name in _REQUIRED_VENV_MODULES if _find_spec(name) is None)


def clean_venv_available() -> bool:
    """Report whether this interpreter can create a clean virtual environment.

    Returns
    -------
    bool
        ``True`` when both ``venv`` and ``ensurepip`` are importable.

    Examples
    --------
    >>> isinstance(clean_venv_available(), bool)
    True

    """
    return not missing_clean_venv_modules()


def clean_venv_skip_reason() -> str | None:
    """Return why a clean virtual environment cannot be created, if it cannot.

    Returns
    -------
    str | None
        A skip message naming every missing standard-library module, or
        ``None`` when ``venv`` and ``ensurepip`` are both importable.

    Examples
    --------
    >>> reason = clean_venv_skip_reason()
    >>> reason is None or "venv" in reason
    True

    """
    missing = missing_clean_venv_modules()
    if not missing:
        return None
    return "clean wheel smoke test requires missing stdlib modules: " + ", ".join(
        missing
    )


def assert_wheel_imports_in_clean_env(wheel_path: Path, work_dir: Path) -> None:
    """Install *wheel_path* into an isolated virtualenv and import it there.

    Parameters
    ----------
    wheel_path:
        Wheel archive built by maturin for the running interpreter.
    work_dir:
        Empty directory outside the checkout used for the environment and the
        import probe.

    Examples
    --------
    >>> assert_wheel_imports_in_clean_env(wheel, work_dir)  # doctest: +SKIP

    """
    environment = work_dir / "venv"
    probe_dir = work_dir / "probe"
    probe_dir.mkdir(parents=True, exist_ok=True)
    _run(
        [sys.executable, "-m", "venv", str(environment)],
        cwd=work_dir,
        timeout=_VENV_CREATION_TIMEOUT_SECONDS,
    )
    python = _venv_python(environment)
    _run(
        [
            str(python),
            "-I",
            "-m",
            "pip",
            # `--isolated` ignores pip environment variables and per-user
            # configuration, so an inherited PIP_TARGET cannot redirect the
            # install outside the virtual environment.
            "--isolated",
            "install",
            "--no-index",
            "--no-deps",
            str(wheel_path.resolve()),
        ],
        cwd=work_dir,
        timeout=_WHEEL_INSTALL_TIMEOUT_SECONDS,
    )
    _run(
        [str(python), "-I", "-c", _CLEAN_IMPORT_PROBE],
        cwd=probe_dir,
        timeout=_IMPORT_PROBE_TIMEOUT_SECONDS,
    )


def _venv_python(environment: Path) -> Path:
    """Return the interpreter path inside a virtual environment."""
    if sys.platform == "win32":
        return environment / "Scripts" / "python.exe"
    return environment / "bin" / "python"


def _clean_environment() -> dict[str, str]:
    """Return an environment without inherited Python or pip configuration."""
    environment = os.environ.copy()
    for name in (
        "PYTHONHOME",
        "PYTHONPATH",
        "PYTHONSTARTUP",
        "PYTHONUSERBASE",
        "VIRTUAL_ENV",
    ):
        environment.pop(name, None)
    # `--isolated` alone still loads global and site-wide pip configuration
    # files. Pointing PIP_CONFIG_FILE at the null device is the documented way
    # to skip them all, so no system override can redirect the install.
    environment["PIP_CONFIG_FILE"] = os.devnull
    return environment


def _run(command: list[str], *, cwd: Path, timeout: int) -> None:
    """Run *command* and fail with captured output when it exits non-zero."""
    # ruff: ignore[subprocess-without-shell-equals-true] the command vector is
    # assembled from the running interpreter, a venv-local interpreter and a
    # repository-local wheel path; no untrusted input reaches the child.
    result = subprocess.run(
        command,
        check=False,
        capture_output=True,
        cwd=cwd,
        env=_clean_environment(),
        text=True,
        timeout=timeout,
    )
    if result.returncode != 0:
        message = (
            f"command failed with exit code {result.returncode}: {command!r}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
        raise AssertionError(message)
