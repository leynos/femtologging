"""Unit tests validating basicConfig behaviour outside BDD scenarios."""

from __future__ import annotations

import pytest

from femtologging import basicConfig, get_logger, reset_manager

# Levels emitted by every case, ordered from most to least verbose so the
# expected sets below read as prefixes of this sequence.
PROBED_LEVELS = ("DEBUG", "INFO", "WARNING", "ERROR")


@pytest.mark.parametrize("force", [True, False])
@pytest.mark.parametrize(
    ("level", "expected_msgs"),
    [
        ("DEBUG", {"debug", "info", "warning", "error"}),
        ("INFO", {"info", "warning", "error"}),
        ("WARNING", {"warning", "error"}),
        ("ERROR", {"error"}),
    ],
)
def test_basic_config_emits_expected_records(
    *,
    force: bool,
    level: str,
    expected_msgs: set[str],
) -> None:
    """Verify that ``basicConfig`` honours level and force combinations."""
    reset_manager()
    basicConfig(level=level, force=force)
    logger = get_logger("root")

    handlers = logger.handler_ptrs_for_test()
    assert len(handlers) == 1, (
        f"basicConfig(level={level!r}, force={force}) must install exactly one "
        f"root handler, but the root logger holds {len(handlers)}"
    )

    records = {
        probed.lower(): logger.log(probed, probed.lower()) for probed in PROBED_LEVELS
    }
    emitted = {name for name, record in records.items() if record is not None}
    assert emitted == expected_msgs, (
        f"basicConfig(level={level!r}, force={force}) must emit exactly "
        f"{sorted(expected_msgs)}, but emitted {sorted(emitted)}"
    )
