"""Run published Python examples whose effects need checking from outside."""

from __future__ import annotations

import typing as typ

from tests.documentation_examples import documented_example, run_example

if typ.TYPE_CHECKING:
    from pathlib import Path


def test_user_guide_flush_example_writes_before_closing(tmp_path: Path) -> None:
    """The published flush example drains its attached file handler."""
    result = run_example(documented_example("flush-handlers-usage"), tmp_path)

    assert result.returncode == 0, f"the published example failed:\n{result.stderr}"
    output = (tmp_path / "service.log").read_text(encoding="utf-8")
    assert "shutting down" in output, "the published example did not flush its record"
