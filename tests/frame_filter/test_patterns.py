"""Unit tests for logging infrastructure patterns."""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import get_logging_infrastructure_patterns

if typ.TYPE_CHECKING:
    from syrupy.assertion import SnapshotAssertion


def test_patterns_returns_expected(snapshot: SnapshotAssertion) -> None:
    """The default infrastructure pattern set is an exact, reviewable contract."""
    # A snapshot pins the whole set, so adding or dropping a pattern is a
    # deliberate, reviewed change rather than an unnoticed behavioural drift.
    assert get_logging_infrastructure_patterns() == snapshot, (
        "default logging infrastructure patterns drifted from the recorded set"
    )


@pytest.mark.parametrize(
    ("filename", "should_match"),
    [
        pytest.param("femtologging/__init__.py", True, id="femtologging-package"),
        pytest.param("_femtologging_rs.cpython-311.so", True, id="rust-extension"),
        pytest.param("logging/__init__.py", True, id="stdlib-logging-init"),
        pytest.param("logging/config.py", True, id="stdlib-logging-config"),
        pytest.param("logging/handlers.py", True, id="stdlib-logging-handlers"),
        pytest.param("<frozen importlib._bootstrap>", True, id="frozen-importlib"),
        pytest.param("myapp/main.py", False, id="application-module"),
        # "logging" alone is deliberately not a pattern, so application modules
        # merely mentioning it must not be mistaken for logging infrastructure.
        pytest.param("myapp/logging_utils.py", False, id="application-logging-helper"),
    ],
)
def test_patterns_match_expected_files(*, filename: str, should_match: bool) -> None:
    """Patterns should match logging infrastructure files and nothing else."""
    patterns = get_logging_infrastructure_patterns()

    matched = [p for p in patterns if p in filename]

    assert bool(matched) is should_match, (
        f"{filename!r} should {'match' if should_match else 'not match'} a logging "
        f"infrastructure pattern; matched={matched} patterns={patterns}"
    )
