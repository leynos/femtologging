"""Pytest plugins shared across the repository."""

from importlib.util import find_spec

pytest_plugins = (
    ("tests.steps.logging_context_steps",)
    if find_spec("pytest_bdd") is not None
    else ()
)
