"""Unit tests validating basicConfig behaviour outside BDD scenarios."""

from __future__ import annotations

import pytest

from femtologging import basicConfig, get_logger, reset_manager


@pytest.mark.parametrize("force", [True, False])
@pytest.mark.parametrize(
    ("level", "expected_msgs", "suppressed_msgs"),
    [
        ("DEBUG", {"debug", "info", "warning", "error"}, set()),
        ("INFO", {"info", "warning", "error"}, {"debug"}),
        ("WARNING", {"warning", "error"}, {"debug", "info"}),
    ],
)
def test_basic_config_emits_expected_records(
    force: bool,
    level: str,
    expected_msgs: set[str],
    suppressed_msgs: set[str],
) -> None:
    """Verify that ``basicConfig`` honours level and force combinations."""
    reset_manager()
    basicConfig(level=level, force=force)
    logger = get_logger("root")
    assert len(logger.handler_ptrs_for_test()) == 1
    records = {
        "debug": logger.log("DEBUG", "debug"),
        "info": logger.log("INFO", "info"),
        "warning": logger.log("WARNING", "warning"),
        "error": logger.log("ERROR", "error"),
    }
    for msg in expected_msgs:
        assert records[msg] is not None
    for msg in suppressed_msgs:
        assert records[msg] is None
