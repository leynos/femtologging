"""Shared BDD steps reused across feature modules."""

from __future__ import annotations

from pytest_bdd import given

from femtologging import reset_manager


@given("the logging system is reset")
def reset_logging() -> None:
    """Reset global logging state for scenario isolation."""
    reset_manager()
