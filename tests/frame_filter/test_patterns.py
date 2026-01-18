"""Unit tests for logging infrastructure patterns."""

from __future__ import annotations

from femtologging import get_logging_infrastructure_patterns


def test_patterns_returns_expected() -> None:
    """Should return the default logging infrastructure patterns."""
    patterns = get_logging_infrastructure_patterns()

    assert "femtologging" in patterns, "femtologging pattern missing"
    assert "logging/__init__" in patterns, "logging/__init__ pattern missing"
    assert "_femtologging_rs" in patterns, "_femtologging_rs pattern missing"


def test_patterns_match_expected_files() -> None:
    """Patterns should match expected logging infrastructure files."""
    patterns = get_logging_infrastructure_patterns()

    # Test that patterns work as expected (substring matching)
    test_filenames = [
        ("femtologging/__init__.py", True),
        ("_femtologging_rs.cpython-311.so", True),
        ("logging/__init__.py", True),
        ("logging/config.py", True),
        ("myapp/main.py", False),
        ("myapp/logging_utils.py", False),  # "logging" is not a pattern
    ]

    for filename, should_match in test_filenames:
        matches = any(p in filename for p in patterns)
        assert matches == should_match, f"{filename}: expected {should_match}"
